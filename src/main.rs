use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, BufReader};
use std::option::Option;
use std::str;
use std::time::Instant;

use rand::Rng;
use rand::distributions::{Alphanumeric, DistString};
use rand::rngs::ThreadRng;

use dotenv;

use serde::{Deserialize, Serialize};
use tiny_http::{Header, HeaderField, Method, Response, Server, StatusCode};


fn main() {
    let _ = dotenv::dotenv();

    //TODO: is there a smarter way to read line by line things?
    let prompts_path = match std::env::var("PROMPTS_PATH") {
        Ok(pp) => pp,
        Err(..) => panic!("Provide a path to read the Prompts"),
    };
    let prompts_string = match fs::read_to_string(prompts_path.clone()) {
        Ok(ps) => ps,
        Err(..) => panic!("Prompts were not found in path '{}'", prompts_path),
    };
    let prompts: Vec<String> = prompts_string.lines().map(String::from).collect();


    let finishers_path = match std::env::var("FINISHERS_PATH") {
        Ok(fp) => fp,
        Err(..) => panic!("Provide a path to read the Finishers"),
    };
    let finishers_string = match fs::read_to_string(finishers_path.clone()) {
        Ok(ps) => ps,
        Err(..) => panic!("Finishers were not found in path '{}'", finishers_path),
    };
    let finishers: Vec<String> = finishers_string.lines().map(String::from).collect();


    let prompt_ids_usize: Vec<usize> = (0..prompts.len()).collect();
    let prompt_ids: Vec<u16> = prompt_ids_usize.iter().map(|&x| x as u16).collect();

    let finisher_ids_usize: Vec<usize> = (0..finishers.len()).collect();
    let finisher_ids: Vec<u16> = finisher_ids_usize.iter().map(|&x| x as u16).collect();

    let host = match std::env::var("HOST") {
        Ok(p) => p,
        Err(..) => "0.0.0.0".to_string(),
    };

    let port = match std::env::var("PORT") {
        Ok(p) => p.parse::<u16>().unwrap(),
        Err(..) => 8000,
    };

    // Server (TPC bind) errors not handled for simplicity
    let host_port = format!("{}:{}", host, port);

    //TODO: SSL
    println!("Starting server at {}.", host_port);
    let server = Server::http(host_port).unwrap();


    let mut rng = rand::thread_rng();
    let mut rooms: HashMap<u32, Room> = HashMap::new();
    let mut players: HashMap<u32, Player> = HashMap::new();
    let mut room_players: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut room_prompts: HashMap<u32, Vec<u16>> = HashMap::new();
    let mut room_finishers: HashMap<u32, HashMap<u32, u16>> = HashMap::new();
    let mut player_finishers: HashMap<u32, Vec<u16>> = HashMap::new();
    let mut room_available_prompts: HashMap<u32, Vec<u16>> = HashMap::new();
    let mut room_available_finishers: HashMap<u32, Vec<u16>> = HashMap::new();
    let mut room_players_not_ready: HashMap<u32, Vec<u32>> = HashMap::new();

    //TODO: is the mutable Game Context stuff thread-safe?
    let game_context = GameContext {
        rng: &mut rng,
        rooms: &mut rooms,
        players: &mut players,
        room_players: &mut room_players,
        room_prompts: &mut room_prompts,
        room_finishers: &mut room_finishers,
        player_finishers: &mut player_finishers,
        room_available_prompts: &mut room_available_prompts,
        room_available_finishers: &mut room_available_finishers,
        room_players_not_ready: &mut room_players_not_ready
    };


    //TODO: multiple workers
    for mut request in server.incoming_requests() {
        // This can be quite noisy with all the RoomCheck requests:
        // println!("received request! method: {:?}, url: {:?}", request.method(), request.url());

        // Logging Headers is noisy:
        // println!("received request! method: {:?}, url: {:?}, headers: {:?}",
        //     request.method(),
        //     request.url(),
        //     request.headers()
        // );

        match get_game_action(request.method(), request.url()) {
            Some(game_action) => {

                // Hack for development
                // Could the headers be a constant?
                let access_control_allow_headers = Header::from_bytes(b"Access-Control-Allow-Headers", b"*").unwrap();
                let access_control_allow_origin_header = Header::from_bytes(b"Access-Control-Allow-Origin", b"*").unwrap();
                let access_control_allow_methods = Header::from_bytes(b"Access-Control-Allow-Methods", b"GET, POST").unwrap();
                let access_control_allow_max_age = Header::from_bytes(b"Access-Control-Max-Age", b"3600").unwrap(); // 3600 = 1 hour
                let headers = Vec::from([access_control_allow_headers, access_control_allow_origin_header, access_control_allow_methods, access_control_allow_max_age]);

                match game_action {
                    GameAction::CorsOption => {
                        let response = Response::new(StatusCode(200), headers, io::empty(), None, None);
                        request.respond(response).unwrap();
                    },
                    GameAction::RoomCreate => {
                        println!("RoomCreate request!");

                        //TODO: I know the code is reapeating A LOT, but got to learn how to use Rust's ownership before I can start doing refactoring things properly
                        // Could the HeaderField be a constant?
                        let content_type_header_field = HeaderField::from_bytes(b"Content-Type").unwrap();
                        let content_type_found = request.headers().iter().find(|&h| h.field == content_type_header_field);
                        if content_type_found.is_none() || (content_type_found.unwrap().value != "application/json" && content_type_found.unwrap().value != "application/json; charset=UTF-8") {
                            println!("RoomCreate - Bad headers");

                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                            request.respond(response).unwrap();
                        } else {

                            let mut content = String::new();
                            let reader = request.as_reader();
                            match reader.read_to_string(&mut content) {
                                Ok(_) => {

                                    match serde_json::from_str::<RequestRoomCreate>(&mut content) {
                                        Ok(deserialized_request) => {

                                            let owner_name = deserialized_request.owner_name;
                                            let trimmed_owner_name = owner_name.trim();

                                            if trimmed_owner_name.is_empty() || trimmed_owner_name.len() > 16 {
                                                println!("RoomCreate - Invalid data");

                                                let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                request.respond(response).unwrap();
                                            } else {

                                                //TODO: Need thread safe id/code generator that doesn't repeat values...
                                                let player_id = game_context.rng.gen();
                                                let player = Player {
                                                    id: player_id,
                                                    name: trimmed_owner_name.to_string(),
                                                    score: 0,
                                                    last_check: Instant::now()
                                                };
                                                game_context.players.insert(player_id, player);

                                                let room_id: u32 = game_context.rng.gen();
                                                let room_code = Alphanumeric.sample_string(game_context.rng, 6).to_uppercase();
                                                let room_code_for_response = room_code.clone();

                                                let room = Room{
                                                    id: room_id,
                                                    code: room_code,
                                                    room_status: RoomStatus::Waiting,
                                                    owner_id: player_id,
                                                    leader_id: player_id,
                                                    round_counter: 1,
                                                    round_total: 10, //TODO: Make Configurable
                                                    leader_player_position: 0,
                                                    selected_prompt_id: None,
                                                    winner_player_id: None,
                                                    winner_finisher_id: None
                                                };
                                                game_context.rooms.insert(room_id, room);

                                                game_context.room_players.insert(room_id, vec![player_id]);

                                                game_context.room_prompts.insert(room_id, vec![]);
                                                game_context.room_finishers.insert(room_id, HashMap::new());

                                                game_context.room_available_prompts.insert(room_id, vec![]);

                                                game_context.room_available_finishers.insert(room_id, vec![]);

                                                game_context.player_finishers.insert(player_id, vec![]);

                                                game_context.room_players_not_ready.insert(room_id, vec![]);

                                                let response_room_create = ResponseRoomCreate { room_id: room_id, room_code: room_code_for_response, player_id: player_id };
                                                let serialized_response = serde_json::to_string(&response_room_create).unwrap();
                                                let response_reader = BufReader::new(serialized_response.as_bytes());
                                                let response = Response::new(StatusCode(201), headers, response_reader, Some(serialized_response.len()), None);
                                                request.respond(response).unwrap();
                                            }

                                        },
                                        Err(_) => {
                                            println!("RoomCreate - Cant read JSON");

                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                            request.respond(response).unwrap();
                                        }
                                    };

                                },
                                Err(_) => {
                                    println!("RoomCreate - Cant read request content");

                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                    request.respond(response).unwrap();
                                }
                            }
                        }
                    },
                    GameAction::RoomJoin => {
                        println!("RoomJoin request!");

                        // Could the HeaderField be a constant?
                        let content_type_header_field = HeaderField::from_bytes(b"Content-Type").unwrap();
                        let content_type_found = request.headers().iter().find(|&h| h.field == content_type_header_field);
                        if content_type_found.is_none() || content_type_found.unwrap().value != "application/json; charset=UTF-8" {
                            println!("RoomJoin - Bad headers");

                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                            request.respond(response).unwrap();
                        } else {

                            let mut content = String::new();
                            let reader = request.as_reader();
                            match reader.read_to_string(&mut content) {
                                Ok(_) => {

                                    match serde_json::from_str::<RequestRoomJoin>(&mut content) {
                                        Ok(deserialized_request) => {

                                            let player_name = deserialized_request.player_name;
                                            let trimmed_player_name = player_name.trim();

                                            let room_code = deserialized_request.room_code;
                                            let trimmed_room_code = room_code.trim();

                                            if trimmed_player_name.is_empty() || trimmed_player_name.len() > 16 || trimmed_room_code.is_empty() || trimmed_room_code.len() > 6 {
                                                println!("RoomJoin - Invalid data");

                                                let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                request.respond(response).unwrap();
                                            } else {

                                                let room_found = game_context.rooms.values().find(|&r| r.code == trimmed_room_code);
                                                if room_found.is_none() {
                                                    println!("RoomJoin - Room '{}' not found", trimmed_room_code);

                                                    let response = Response::new(StatusCode(404), headers, io::empty(), None, None);
                                                    request.respond(response).unwrap();
                                                } else {
                                                    let room_id = room_found.unwrap().id;

                                                    //TODO: Join player: check player name is not already in the Room
                                                    //TODO: Validate the room is in the context, otherwise 500
                                                    //TODO: Validate max number of players
                                                    let players_in_room = game_context.room_players.get_mut(&room_id).unwrap();


                                                    //TODO: Need thread safe id/code generator that doesn't repeat values...
                                                    let player_id = game_context.rng.gen();
                                                    let player = Player {
                                                        id: player_id,
                                                        name: trimmed_player_name.to_string(),
                                                        score: 0,
                                                        last_check: Instant::now()
                                                    };
                                                    game_context.players.insert(player_id, player);

                                                    game_context.player_finishers.insert(player_id, vec![]);

                                                    players_in_room.push(player_id);

                                                    let response_room_join = ResponseRoomJoin { room_id: room_id, player_id: player_id };
                                                    let serialized_response = serde_json::to_string(&response_room_join).unwrap();
                                                    let response_reader = BufReader::new(serialized_response.as_bytes());
                                                    let response = Response::new(StatusCode(200), headers, response_reader, Some(serialized_response.len()), None);
                                                    request.respond(response).unwrap();
                                                }
                                            }

                                        },
                                        Err(_) => {
                                            println!("RoomJoin - Cant read JSON");

                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                            request.respond(response).unwrap();
                                        }
                                    };

                                },
                                Err(_) => {
                                    println!("RoomJoin - Cant read request content");

                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                    request.respond(response).unwrap();
                                }
                            }
                        }
                    },
                    GameAction::RoomCheck => {
                        // println!("RoomCheck request!");

                        // Could the HeaderField be a constant?
                        let content_type_header_field = HeaderField::from_bytes(b"Content-Type").unwrap();
                        let content_type_found = request.headers().iter().find(|&h| h.field == content_type_header_field);
                        if content_type_found.is_none() || content_type_found.unwrap().value != "application/json; charset=UTF-8" {
                            println!("RoomCheck - Bad headers");

                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                            request.respond(response).unwrap();
                        } else {

                            let mut content = String::new();
                            let reader = request.as_reader();
                            match reader.read_to_string(&mut content) {
                                Ok(_) => {

                                    match serde_json::from_str::<RequestRoomCheck>(&mut content) {
                                        Ok(deserialized_request) => {

                                            let room_id = deserialized_request.room_id;

                                            let player_id = deserialized_request.player_id;

                                            if room_id == 0 || player_id == 0 {
                                                println!("RoomCheck - Invalid data");

                                                let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                request.respond(response).unwrap();
                                            } else {

                                                let room_found = game_context.rooms.get(&room_id);
                                                if room_found.is_none() {
                                                    println!("RoomCheck - Room {} not found", room_id);

                                                    let response = Response::new(StatusCode(404), headers, io::empty(), None, None);
                                                    request.respond(response).unwrap();
                                                } else {
                                                    let players_in_room = game_context.room_players.get(&room_id).unwrap();

                                                    let player_found = players_in_room.iter().find(|&p_id| p_id == &player_id);
                                                    if player_found.is_none() {
                                                        println!("RoomCheck - Player {} not found in room {}", player_id, room_id);

                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                        request.respond(response).unwrap();
                                                    } else {
                                                        let player_optional = game_context.players.get_mut(&player_id);
                                                        if player_optional.is_none() {
                                                            println!("RoomCheck - Player {} not found!!!", player_id);

                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                            request.respond(response).unwrap();
                                                        } else {
                                                            let room = room_found.unwrap();

                                                            let player = player_optional.unwrap();
                                                            player.last_check = Instant::now();

                                                            let room_players_not_ready = game_context.room_players_not_ready.get(&room_id).unwrap();

                                                            let players_in_room_response = players_in_room.iter().map(|room_player_id| {
                                                                //TODO: Validate the user is in the context.users(?), otherwise 500
                                                                let room_player = game_context.players.get(&room_player_id).unwrap();
                                                                let is_finisher_ready: Option<bool>;
                                                                if room.leader_id == room_player.id {
                                                                    is_finisher_ready = None;
                                                                } else {
                                                                    let room_finishers = game_context.room_finishers.get(&room_id).unwrap();
                                                                    if room_finishers.get(room_player_id).is_none() {
                                                                        is_finisher_ready = Some(false);
                                                                    } else {
                                                                        is_finisher_ready = Some(true);
                                                                    }
                                                                }

                                                                let is_player_next_round_ready: Option<bool>;
                                                                match &room.room_status {
                                                                    RoomStatus::RoundWinner => {
                                                                        is_player_next_round_ready = Some(!room_players_not_ready.contains(&room_player_id));
                                                                    },
                                                                    _ => {
                                                                        is_player_next_round_ready = None;
                                                                    }
                                                                }

                                                                ResponseRoomCheckPlayer {
                                                                    player_id: room_player.id,
                                                                    player_name: room_player.name.to_string(),
                                                                    score: room_player.score,
                                                                    is_finisher_ready: is_finisher_ready,
                                                                    is_next_round_ready: is_player_next_round_ready,
                                                                    last_check: u16::try_from(room_player.last_check.elapsed().as_secs()).unwrap()
                                                                }
                                                            }).collect();

                                                            let room_status = room.room_status.to_string();

                                                            let response_prompt_text: Option<String>;
                                                            let response_finishers: Option<Vec<ResponseRoomCheckFinisher>>;
                                                            match &room.room_status {
                                                                RoomStatus::LackeyOptions => {
                                                                    let prompt_position = room.selected_prompt_id.unwrap();
                                                                    let prompt_position_usize = usize::try_from(prompt_position).unwrap();
                                                                    response_prompt_text = Some(prompts[prompt_position_usize].clone());

                                                                    response_finishers = None;
                                                                },
                                                                RoomStatus::LeaderPick => {
                                                                    let prompt_position = room.selected_prompt_id.unwrap();
                                                                    let prompt_position_usize = usize::try_from(prompt_position).unwrap();
                                                                    response_prompt_text = Some(prompts[prompt_position_usize].clone());

                                                                    response_finishers = None;
                                                                },
                                                                RoomStatus::RoundWinner => {
                                                                    let prompt_position = room.selected_prompt_id.unwrap();
                                                                    let prompt_position_usize = usize::try_from(prompt_position).unwrap();
                                                                    response_prompt_text = Some(prompts[prompt_position_usize].clone());


                                                                    let room_finishers = game_context.room_finishers.get(&room_id).unwrap();
                                                                    let converted_finishers = room_finishers.iter().map(|(&player_id, &finisher_id)| {

                                                                        let finisher_position_usize = usize::try_from(finisher_id).unwrap();
                                                                        let finisher = &finishers[finisher_position_usize];

                                                                        let player = game_context.players.get(&player_id).unwrap();

                                                                        ResponseRoomCheckFinisher {
                                                                            player_name: player.name.clone(),
                                                                            finisher_text: finisher.to_string(),
                                                                            is_winner: (player_id == room.winner_player_id.unwrap())
                                                                        }
                                                                    }).collect();

                                                                    response_finishers = Some(converted_finishers);
                                                                },
                                                                _ => {
                                                                    response_prompt_text = None;
                                                                    response_finishers = None;
                                                                }
                                                            }


                                                            let response_room_create = ResponseRoomCheck {
                                                                players: players_in_room_response,
                                                                room_status: room_status,
                                                                owner_id: room.owner_id,
                                                                leader_id: room.leader_id,
                                                                round_counter: room.round_counter,
                                                                round_total: room.round_total,
                                                                prompt_text: response_prompt_text,
                                                                finishers: response_finishers
                                                            };
                                                            let serialized_response = serde_json::to_string(&response_room_create).unwrap();
                                                            let response_reader = BufReader::new(serialized_response.as_bytes());
                                                            let response = Response::new(StatusCode(201), headers, response_reader, Some(serialized_response.len()), None);
                                                            request.respond(response).unwrap();
                                                        }
                                                    }
                                                }
                                            }

                                        },
                                        Err(_) => {
                                            println!("RoomCheck - Cant read JSON");

                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                            request.respond(response).unwrap();
                                        }
                                    };

                                },
                                Err(_) => {
                                    println!("RoomCheck - Cant read request content");

                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                    request.respond(response).unwrap();
                                }
                            }
                        }
                    },
                    GameAction::GameStart => {
                        println!("GameStart request!");

                        // Could the HeaderField be a constant?
                        let content_type_header_field = HeaderField::from_bytes(b"Content-Type").unwrap();
                        let content_type_found = request.headers().iter().find(|&h| h.field == content_type_header_field);
                        if content_type_found.is_none() || content_type_found.unwrap().value != "application/json; charset=UTF-8" {
                            println!("GameStart - Bad headers");

                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                            request.respond(response).unwrap();
                        } else {

                            let mut content = String::new();
                            let reader = request.as_reader();
                            match reader.read_to_string(&mut content) {
                                Ok(_) => {

                                    match serde_json::from_str::<RequestGameStart>(&mut content) {
                                        Ok(deserialized_request) => {

                                            let room_id = deserialized_request.room_id;

                                            let player_id = deserialized_request.player_id;

                                            if room_id == 0 || player_id == 0 {
                                                println!("GameStart - Invalid data");

                                                let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                request.respond(response).unwrap();
                                            } else {

                                                let room_found = game_context.rooms.get_mut(&room_id);
                                                if room_found.is_none() {
                                                    println!("GameStart - Room {} not found", room_id);
                                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                    request.respond(response).unwrap();
                                                } else {

                                                    let players_in_room = game_context.room_players.get(&room_id).unwrap();
                                                    let room_player_count = u8::try_from(players_in_room.len()).unwrap();
                                                    if room_player_count <= 2 {
                                                        println!("GameStart - Not enough players in room ({})", room_player_count);

                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                        request.respond(response).unwrap();
                                                    } else {
                                                        let player_found = players_in_room.iter().find(|&p_id| p_id == &player_id);
                                                        if player_found.is_none() {
                                                            println!("GameStart - Player {} not found in room {}", player_id, room_id);

                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                            request.respond(response).unwrap();
                                                        } else {
                                                            let player_optional = game_context.players.get(&player_id);
                                                            if player_optional.is_none() {
                                                                println!("GameStart - Player {} not found!!!", player_id);

                                                                let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                request.respond(response).unwrap();
                                                            } else {
                                                                let room = room_found.unwrap();

                                                                match room.room_status {
                                                                    RoomStatus::Waiting => {
                                                                        if player_id != room.owner_id {
                                                                            println!("GameStart - Player {} is not the owner of the room", player_id);

                                                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        } else {
                                                                            // New game and round
                                                                            room.room_status = RoomStatus::LeaderOptions;

                                                                            // No need to set the round_counter or leader_player_position.
                                                                            // The default values with which the Room was created are fine.

                                                                            let response = Response::new(StatusCode(204), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        }
                                                                    },
                                                                    RoomStatus::RoundWinner => {
                                                                        // Old game but new round

                                                                        let room_players_not_ready_found = game_context.room_players_not_ready.get_mut(&room_id);
                                                                        if room_players_not_ready_found.is_none() {
                                                                            println!("GameStart - Room {} of Players Ready not found", room_id);

                                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        } else {
                                                                            let room_players_not_ready = room_players_not_ready_found.unwrap();
                                                                            room_players_not_ready.retain(|&p_id| p_id != player_id);

                                                                            if room_players_not_ready.is_empty() {
                                                                                // All players are ready

                                                                                room.selected_prompt_id = None;
                                                                                room.winner_player_id = None;
                                                                                room.winner_finisher_id = None;

                                                                                if room.round_counter < room.round_total {
                                                                                    // Next round

                                                                                    // Clear Prompts
                                                                                    game_context.room_prompts.get_mut(&room_id).unwrap().clear();

                                                                                    room.round_counter += 1;
                                                                                    room.leader_player_position = (room.leader_player_position + 1) % room_player_count;

                                                                                    let player_position_usize = usize::try_from(room.leader_player_position).unwrap();
                                                                                    room.leader_id = players_in_room[player_position_usize];

                                                                                    room.room_status = RoomStatus::LeaderOptions;

                                                                                    let response = Response::new(StatusCode(204), headers, io::empty(), None, None);
                                                                                    request.respond(response).unwrap();
                                                                                } else {
                                                                                    // Game end

                                                                                    // Leader and turn changes happen later during GameWinner.

                                                                                    room.room_status = RoomStatus::GameWinner;

                                                                                    let response = Response::new(StatusCode(204), headers, io::empty(), None, None);
                                                                                    request.respond(response).unwrap();
                                                                                }

                                                                            } else {
                                                                                // Not all players are ready

                                                                                let response = Response::new(StatusCode(204), headers, io::empty(), None, None);
                                                                                request.respond(response).unwrap();
                                                                            }
                                                                        }
                                                                    },
                                                                    RoomStatus::GameWinner => {
                                                                        // Start a new Game

                                                                        if player_id != room.owner_id {
                                                                            println!("GameStart - Player {} is not the owner of the room", player_id);

                                                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        } else {

                                                                            // Reset all the player scores
                                                                            let mut reset_ok = true;
                                                                            for room_player_id in players_in_room {
                                                                                let room_player_optional = game_context.players.get_mut(&room_player_id);
                                                                                if room_player_optional.is_none() {
                                                                                    println!("GameStart - Player {} not found!!!", room_player_id);

                                                                                    reset_ok = false;
                                                                                    break;
                                                                                } else {
                                                                                    let room_player = room_player_optional.unwrap();
                                                                                    room_player.score = 0;
                                                                                }
                                                                            }

                                                                            if reset_ok {
                                                                                // Clear Prompts
                                                                                game_context.room_prompts.get_mut(&room_id).unwrap().clear();

                                                                                room.leader_player_position = (room.leader_player_position + 1) % room_player_count;

                                                                                let player_position_usize = usize::try_from(room.leader_player_position).unwrap();
                                                                                room.leader_id = players_in_room[player_position_usize];

                                                                                room.room_status = RoomStatus::LeaderOptions;
                                                                                room.round_counter = 1;
                                                                                room.selected_prompt_id = None;
                                                                                room.winner_player_id = None;
                                                                                room.winner_finisher_id = None;

                                                                                let response = Response::new(StatusCode(204), headers, io::empty(), None, None);
                                                                                request.respond(response).unwrap();
                                                                            } else {
                                                                                println!("GameStart - Reset failed!!!");
                                                                                let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                                request.respond(response).unwrap();
                                                                            }
                                                                        }
                                                                    }
                                                                    _ => {
                                                                        println!("GameStart - Room not waiting");
                                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                        request.respond(response).unwrap();
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }

                                        },
                                        Err(_) => {
                                            println!("GameStart - Cant read JSON");

                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                            request.respond(response).unwrap();
                                        }
                                    };

                                },
                                Err(_) => {
                                    println!("GameStart - Cant read request content");

                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                    request.respond(response).unwrap();
                                }
                            }
                        }
                    },
                    GameAction::GameOptions => {
                        println!("GameOptions request!");

                        // Could the HeaderField be a constant?
                        let content_type_header_field = HeaderField::from_bytes(b"Content-Type").unwrap();
                        let content_type_found = request.headers().iter().find(|&h| h.field == content_type_header_field);
                        if content_type_found.is_none() || content_type_found.unwrap().value != "application/json; charset=UTF-8" {
                            println!("GameOptions - Bad headers");

                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                            request.respond(response).unwrap();
                        } else {

                            let mut content = String::new();
                            let reader = request.as_reader();
                            match reader.read_to_string(&mut content) {
                                Ok(_) => {

                                    match serde_json::from_str::<RequestGameOptions>(&mut content) {
                                        Ok(deserialized_request) => {

                                            let room_id = deserialized_request.room_id;

                                            let player_id = deserialized_request.player_id;

                                            if room_id == 0 || player_id == 0 {
                                                println!("GameOptions - Invalid data");

                                                let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                request.respond(response).unwrap();
                                            } else {

                                                let room_found = game_context.rooms.get(&room_id);
                                                if room_found.is_none() {
                                                    println!("GameOptions - Room {} not found", room_id);

                                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                    request.respond(response).unwrap();
                                                } else {
                                                    let players_in_room = game_context.room_players.get(&room_id).unwrap();

                                                    let player_found = players_in_room.iter().find(|&p_id| p_id == &player_id);
                                                    if player_found.is_none() {
                                                        println!("GameOptions - Player {} not found in room {}", player_id, room_id);

                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                        request.respond(response).unwrap();
                                                    } else {
                                                        let player_optional = game_context.players.get_mut(&player_id);
                                                        if player_optional.is_none() {
                                                            println!("GameOptions - Player {} not found!!!", player_id);

                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                            request.respond(response).unwrap();
                                                        } else {
                                                            let room = room_found.unwrap();

                                                            if room.leader_id == player_id {
                                                                match &room.room_status {
                                                                    RoomStatus::LeaderOptions => {
                                                                        // Prompts
                                                                        let room_prompts_optional = game_context.room_prompts.get_mut(&room_id);
                                                                        if room_prompts_optional.is_none() {
                                                                            println!("GameOptions - Room {} prompts not found", room_id);

                                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        } else {
                                                                            let room_prompts = room_prompts_optional.unwrap();

                                                                            let room_prompt_count = room_prompts.len();
                                                                            for _ in room_prompt_count..3 {
                                                                                let room_available_prompts = game_context.room_available_prompts.get_mut(&room_id).unwrap();
                                                                                let random_prompt = room_available_prompts.pop();
                                                                                if random_prompt.is_none() {
                                                                                    // Refill (no need to exclude any)

                                                                                    // A hacky random vector generator
                                                                                    let mut prompt_ids_in = prompt_ids.clone();
                                                                                    let mut prompt_ids_out = vec!();
                                                                                    while !prompt_ids_in.is_empty() {
                                                                                        prompt_ids_out.push(prompt_ids_in.remove(game_context.rng.gen_range(0..prompt_ids_in.len())));
                                                                                    }

                                                                                    room_available_prompts.append(&mut prompt_ids_out);

                                                                                    let new_random_prompt = room_available_prompts.pop().unwrap();
                                                                                    room_prompts.push(new_random_prompt);
                                                                                } else {
                                                                                    room_prompts.push(random_prompt.unwrap());
                                                                                }
                                                                            }

                                                                            let options = room_prompts.iter().map(|&prompt_position| {
                                                                                let prompt_position_usize = usize::try_from(prompt_position).unwrap();
                                                                                let prompt = &prompts[prompt_position_usize];
                                                                                ResponseGameOptionsOption {
                                                                                    option_id: prompt_position,
                                                                                    option_text: prompt.to_string()
                                                                                }
                                                                            }).collect();

                                                                            let response_game_options = ResponseGameOptions { options: options };
                                                                            let serialized_response = serde_json::to_string(&response_game_options).unwrap();
                                                                            let response_reader = BufReader::new(serialized_response.as_bytes());
                                                                            let response = Response::new(StatusCode(200), headers, response_reader, Some(serialized_response.len()), None);
                                                                            request.respond(response).unwrap();
                                                                        }
                                                                    }
                                                                    RoomStatus::LeaderPick => {
                                                                        // Finishers
                                                                        let room_finishers_optional = game_context.room_finishers.get_mut(&room_id);
                                                                        if room_finishers_optional.is_none() {
                                                                            println!("GameOptions - Room {} finishers not found", room_id);

                                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        } else {
                                                                            let room_finishers = room_finishers_optional.unwrap();

                                                                            let options = room_finishers.values().map(|&finisher_position| {
                                                                                let finisher_position_usize = usize::try_from(finisher_position).unwrap();
                                                                                let finisher = &finishers[finisher_position_usize];
                                                                                ResponseGameOptionsOption {
                                                                                    option_id: finisher_position,
                                                                                    option_text: finisher.to_string()
                                                                                }
                                                                            }).collect();

                                                                            let response_game_options = ResponseGameOptions { options: options };
                                                                            let serialized_response = serde_json::to_string(&response_game_options).unwrap();
                                                                            let response_reader = BufReader::new(serialized_response.as_bytes());
                                                                            let response = Response::new(StatusCode(200), headers, response_reader, Some(serialized_response.len()), None);
                                                                            request.respond(response).unwrap();
                                                                        }
                                                                    }
                                                                    _ => {
                                                                        println!("GameOptions - Player {} requested prompts on the wrong room status {}", player_id, room.room_status);

                                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                        request.respond(response).unwrap();
                                                                    }
                                                                }
                                                            } else {
                                                                match &room.room_status {
                                                                    RoomStatus::LackeyOptions => {
                                                                        // Finishers
                                                                        let player_finishers_optional = game_context.player_finishers.get(&player_id);
                                                                        if player_finishers_optional.is_none() {
                                                                            println!("GameOptions - Player {} finishers not found", player_id);

                                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        } else {
                                                                            let player_finishers = player_finishers_optional.unwrap();

                                                                            let player_finisher_count = player_finishers.len();
                                                                            for _ in player_finisher_count..8 {

                                                                                let room_available_finishers = game_context.room_available_finishers.get_mut(&room_id).unwrap();
                                                                                let random_finisher = room_available_finishers.pop();
                                                                                if random_finisher.is_none() {
                                                                                    let mut temp_finishers: Vec<u16> = vec![];
                                                                                    for temp_player_id in game_context.room_players.get(&room_id).unwrap() {
                                                                                        for temp_finisher_id in game_context.player_finishers.get(temp_player_id).unwrap() {
                                                                                            temp_finishers.push(*temp_finisher_id);
                                                                                        }
                                                                                    }

                                                                                    // Refill

                                                                                    // A hacky random vector generator
                                                                                    let mut finisher_ids_in = finisher_ids.clone();
                                                                                    let mut finisher_ids_out = vec!();
                                                                                    while !finisher_ids_in.is_empty() {
                                                                                        finisher_ids_out.push(finisher_ids_in.remove(game_context.rng.gen_range(0..finisher_ids_in.len())));
                                                                                    }

                                                                                    room_available_finishers.append(&mut finisher_ids_out);

                                                                                    let new_random_finisher = room_available_finishers.pop().unwrap();

                                                                                    let player_finishers_mutable = game_context.player_finishers.get_mut(&player_id).unwrap();
                                                                                    player_finishers_mutable.push(new_random_finisher);
                                                                                } else {
                                                                                    let player_finishers_mutable = game_context.player_finishers.get_mut(&player_id).unwrap();
                                                                                    player_finishers_mutable.push(random_finisher.unwrap());
                                                                                }
                                                                            }

                                                                            let player_finishers_again = game_context.player_finishers.get(&player_id).unwrap();
                                                                            let options = player_finishers_again.iter().map(|&finisher_position| {
                                                                                let finisher_position_usize = usize::try_from(finisher_position).unwrap();
                                                                                let finisher = &finishers[finisher_position_usize];
                                                                                ResponseGameOptionsOption {
                                                                                    option_id: finisher_position,
                                                                                    option_text: finisher.to_string()
                                                                                }
                                                                            }).collect();

                                                                            let response_game_options = ResponseGameOptions { options: options };
                                                                            let serialized_response = serde_json::to_string(&response_game_options).unwrap();
                                                                            let response_reader = BufReader::new(serialized_response.as_bytes());
                                                                            let response = Response::new(StatusCode(200), headers, response_reader, Some(serialized_response.len()), None);
                                                                            request.respond(response).unwrap();
                                                                        }
                                                                    }
                                                                    _ => {
                                                                        println!("GameOptions - Player {} requested finishers on the wrong room status {}", player_id, room.room_status);

                                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                        request.respond(response).unwrap();
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }

                                        },
                                        Err(_) => {
                                            println!("GameOptions - Cant read JSON");

                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                            request.respond(response).unwrap();
                                        }
                                    };

                                },
                                Err(_) => {
                                    println!("GameOptions - Cant read request content");

                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                    request.respond(response).unwrap();
                                }
                            }
                        }
                    },
                    GameAction::GamePick => {
                        println!("GamePick request!");

                        // Could the HeaderField be a constant?
                        let content_type_header_field = HeaderField::from_bytes(b"Content-Type").unwrap();
                        let content_type_found = request.headers().iter().find(|&h| h.field == content_type_header_field);
                        if content_type_found.is_none() || content_type_found.unwrap().value != "application/json; charset=UTF-8" {
                            println!("GamePick - Bad headers");

                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                            request.respond(response).unwrap();
                        } else {

                            let mut content = String::new();
                            let reader = request.as_reader();
                            match reader.read_to_string(&mut content) {
                                Ok(_) => {

                                    match serde_json::from_str::<RequestGamePick>(&mut content) {
                                        Ok(deserialized_request) => {

                                            let room_id = deserialized_request.room_id;

                                            let player_id = deserialized_request.player_id;

                                            let option_id = deserialized_request.option_id;

                                            // No need to validate the option_id as it can be zero!
                                            if room_id == 0 || player_id == 0 {
                                                println!("GamePick - Invalid data");

                                                let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                request.respond(response).unwrap();
                                            } else {

                                                let room_found = game_context.rooms.get_mut(&room_id);
                                                if room_found.is_none() {
                                                    println!("GamePick - Room {} not found", room_id);

                                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                    request.respond(response).unwrap();
                                                } else {
                                                    let players_in_room = game_context.room_players.get(&room_id).unwrap();

                                                    let player_found = players_in_room.iter().find(|&p_id| p_id == &player_id);
                                                    if player_found.is_none() {
                                                        println!("GamePick - Player {} not found in room {}", player_id, room_id);

                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                        request.respond(response).unwrap();
                                                    } else {
                                                        let player_optional = game_context.players.get_mut(&player_id);
                                                        if player_optional.is_none() {
                                                            println!("GamePick - Player {} not found!!!", player_id);

                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                            request.respond(response).unwrap();
                                                        } else {
                                                            let room = room_found.unwrap();

                                                            if room.leader_id == player_id {
                                                                match &room.room_status {
                                                                    RoomStatus::LeaderOptions => {
                                                                        // Prompts
                                                                        let room_prompts_optional = game_context.room_prompts.get(&room_id);
                                                                        if room_prompts_optional.is_none() {
                                                                            println!("GamePick - Player {} leader prompts not found", player_id);

                                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        } else {
                                                                            let room_prompts = room_prompts_optional.unwrap();

                                                                            let player_prompt_found = room_prompts.iter().find(|&pr_id| pr_id == &option_id);
                                                                            if player_prompt_found.is_none() {
                                                                                println!("GamePick - Player {} leader prompt {} not found", player_id, option_id);

                                                                                let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                                request.respond(response).unwrap();
                                                                            } else {

                                                                                let room_finishers_optional = game_context.room_finishers.get_mut(&room_id);
                                                                                if room_finishers_optional.is_none() {
                                                                                    println!("GamePick - Room {} finishers not found", room_id);

                                                                                    let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                                    request.respond(response).unwrap();
                                                                                } else {
                                                                                    // Need to clean it so that the lackeys can place new cards on it
                                                                                    room_finishers_optional.unwrap().clear();

                                                                                    room.room_status = RoomStatus::LackeyOptions;
                                                                                    room.selected_prompt_id = Some(option_id);

                                                                                    let response = Response::new(StatusCode(204), headers, io::empty(), None, None);
                                                                                    request.respond(response).unwrap();
                                                                                }
                                                                            }
                                                                        }
                                                                    },
                                                                    RoomStatus::LeaderPick => {
                                                                        let room_finishers_optional = game_context.room_finishers.get_mut(&room_id);
                                                                        if room_finishers_optional.is_none() {
                                                                            println!("GamePick - Room {} finishers not found", room_id);

                                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        } else {
                                                                            let room_finishers = room_finishers_optional.unwrap();

                                                                            let player_finisher_found = room_finishers.iter().find(|(_, &val)| val == option_id);
                                                                            if player_finisher_found.is_none() {
                                                                                println!("GamePick - Player {} finisher prompt {} not found", player_id, option_id);

                                                                                let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                                request.respond(response).unwrap();
                                                                            } else {
                                                                                let player_finisher = player_finisher_found.unwrap();

                                                                                let winner_player_id = player_finisher.0;
                                                                                let winner_player_optional = game_context.players.get_mut(&winner_player_id);
                                                                                if winner_player_optional.is_none() {
                                                                                    println!("GamePick - Player {} not found", winner_player_id);

                                                                                    let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                                    request.respond(response).unwrap();
                                                                                } else {
                                                                                    let winner_player = winner_player_optional.unwrap();

                                                                                    let room_players_not_ready_found = game_context.room_players_not_ready.get_mut(&room_id);
                                                                                    if room_players_not_ready_found.is_none() {
                                                                                        println!("GamePick - Room {} of Players not Ready not found", room_id);

                                                                                        let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                                        request.respond(response).unwrap();
                                                                                    } else {
                                                                                        // No player is ready now (including leader)
                                                                                        let mut clone = players_in_room.clone();
                                                                                        room_players_not_ready_found.unwrap().append(&mut clone);


                                                                                        winner_player.score += 1;

                                                                                        room.room_status = RoomStatus::RoundWinner;
                                                                                        room.winner_player_id = Some(*winner_player_id);
                                                                                        room.winner_finisher_id = Some(option_id);

                                                                                        let response = Response::new(StatusCode(204), headers, io::empty(), None, None);
                                                                                        request.respond(response).unwrap();
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    },
                                                                    _ => {
                                                                        println!("GamePick - Player {} leader picked an option on the wrong room status {}", player_id, room.room_status);

                                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                        request.respond(response).unwrap();
                                                                    }
                                                                }
                                                            } else {
                                                                match &room.room_status {
                                                                    RoomStatus::LackeyOptions => {
                                                                        // Finishers
                                                                        let player_finishers_optional = game_context.player_finishers.get_mut(&player_id);
                                                                        if player_finishers_optional.is_none() {
                                                                            println!("GameOptions - Player {} lackey finishers not found", player_id);

                                                                            let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                            request.respond(response).unwrap();
                                                                        } else {
                                                                            let player_finishers = player_finishers_optional.unwrap();

                                                                            let player_finisher_found = player_finishers.iter().find(|&f_id| f_id == &option_id);
                                                                            if player_finisher_found.is_none() {
                                                                                println!("GamePick - Player {} lackey finisher {} not found", player_id, option_id);

                                                                                let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                                request.respond(response).unwrap();
                                                                            } else {
                                                                                let room_finishers_optional = game_context.room_finishers.get_mut(&room_id);
                                                                                if room_finishers_optional.is_none() {
                                                                                    println!("GamePick - Room {} finishers not found", room_id);

                                                                                    let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                                                                                    request.respond(response).unwrap();
                                                                                } else {
                                                                                    let room_finishers = room_finishers_optional.unwrap();

                                                                                    let room_finisher_player_found = room_finishers.get(&player_id);
                                                                                    if room_finisher_player_found.is_none() {
                                                                                        //TODO: This operation here should be atomic to prevent weird game states...
                                                                                        player_finishers.retain(|&f| f != option_id);

                                                                                        room_finishers.insert(player_id, option_id);

                                                                                        let mut all_players_submitted_finishers = true;
                                                                                        for room_player_id in players_in_room {
                                                                                            if room_player_id != &room.leader_id {
                                                                                                let player_submitted_finisher = room_finishers.get(room_player_id);
                                                                                                if player_submitted_finisher.is_none() {
                                                                                                    all_players_submitted_finishers = false;
                                                                                                    break;
                                                                                                }
                                                                                            }
                                                                                        }

                                                                                        //TODO: What about dead clients?
                                                                                        //      Do we wait for the job to disconnect them?
                                                                                        //      Or should they just be ignored during the check?
                                                                                        //TODO: Can we have the owner or leader force things?
                                                                                        if all_players_submitted_finishers {
                                                                                            room.room_status = RoomStatus::LeaderPick;
                                                                                        }

                                                                                        let response = Response::new(StatusCode(204), headers, io::empty(), None, None);
                                                                                        request.respond(response).unwrap();
                                                                                    } else {
                                                                                        println!("GamePick - Player {} lackey finisher already submitted", player_id);

                                                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                                        request.respond(response).unwrap();
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    },
                                                                    _ => {
                                                                        println!("GameOptions - Player {} lackey picked a finisher on the wrong room status {}", player_id, room.room_status);

                                                                        let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                                        request.respond(response).unwrap();
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }

                                        },
                                        Err(_) => {
                                            println!("GameOptions - Cant read JSON");

                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                            request.respond(response).unwrap();
                                        }
                                    };

                                },
                                Err(_) => {
                                    println!("GameOptions - Cant read request content");

                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                    request.respond(response).unwrap();
                                }
                            }
                        }
                    }
                    // _ => {
                    //     println!("Unknown Request");

                    //     let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                    //     request.respond(response).unwrap();
                    // }
                }

            },
            None => request.respond(Response::empty(400)).unwrap()
        };
    }

    println!("Shutting down.");
}

fn get_game_action(method: &Method, url: &str) -> Option<GameAction> {
    match method {
        // Method::Get => match url {
        //     _ => return None
        // },
        Method::Post => match url {
            "/room-create" => return Some(GameAction::RoomCreate),
            "/room-join" => return Some(GameAction::RoomJoin),
            //TODO: I know that the Room Check should be a GET, but I don't want to parse the Request's  URL parameters manually.
            "/room-check" => return Some(GameAction::RoomCheck),
            "/game-start" => return Some(GameAction::GameStart),
            //TODO: I know that the Game Status should be a GET, but I don't want to parse the Request's  URL parameters manually.
            "/game-options" => return Some(GameAction::GameOptions),
            "/game-pick" => return Some(GameAction::GamePick),
            _ => return None
        },
        Method::Options => return Some(GameAction::CorsOption),
        _ => return None
    };
}

impl fmt::Display for RoomStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RoomStatus::Waiting => write!(f, "WAITING"),
            RoomStatus::LeaderOptions => write!(f, "LEADER_OPTIONS"),
            RoomStatus::LackeyOptions => write!(f, "LACKEY_OPTIONS"),
            RoomStatus::LeaderPick => write!(f, "LEADER_PICK"),
            RoomStatus::RoundWinner => write!(f, "ROUND_WINNER"),
            RoomStatus::GameWinner => write!(f, "GAME_WINNER")
        }
    }
}


struct GameContext<'a> {
    rng: &'a mut ThreadRng,
    rooms: &'a mut HashMap<u32, Room>,
    players: &'a mut HashMap<u32, Player>,
    room_players: &'a mut HashMap<u32, Vec<u32>>,
    room_prompts: &'a mut HashMap<u32, Vec<u16>>,
    room_finishers: &'a mut HashMap<u32, HashMap<u32, u16>>, // Inner map: PlayerId, FinisherId
    player_finishers: &'a mut HashMap<u32, Vec<u16>>,
    room_available_prompts: &'a mut HashMap<u32, Vec<u16>>,
    room_available_finishers: &'a mut HashMap<u32, Vec<u16>>,
    room_players_not_ready: &'a mut HashMap<u32, Vec<u32>>
}

enum GameAction {
    CorsOption,
    RoomCreate,
    RoomJoin,
    RoomCheck,
    GameStart,
    GameOptions,
    GamePick
}

struct Room {
    id: u32,
    code: String,
    room_status: RoomStatus,
    owner_id: u32,
    leader_id: u32,
    round_counter: u8,
    round_total: u8,
    leader_player_position: u8,
    selected_prompt_id: Option<u16>,
    winner_player_id: Option<u32>,
    winner_finisher_id: Option<u16>
}

enum RoomStatus {
    Waiting,
    LeaderOptions,
    LackeyOptions,
    LeaderPick,
    RoundWinner,
    GameWinner
}

struct Player {
    id: u32,
    name: String,
    score: u8,
    last_check: Instant
}


#[derive(Deserialize, Debug)]
struct RequestRoomCreate {
    owner_name: String
}

#[derive(Serialize, Debug)]
struct ResponseRoomCreate {
    room_id: u32,
    room_code: String,
    player_id: u32
}


#[derive(Deserialize, Debug)]
struct RequestRoomJoin {
    player_name: String,
    room_code: String
}

#[derive(Serialize, Debug)]
struct ResponseRoomJoin {
    room_id: u32,
    player_id: u32
}


#[derive(Deserialize, Debug)]
struct RequestRoomCheck {
    room_id: u32,
    player_id: u32
}

#[derive(Serialize, Debug)]
struct ResponseRoomCheck {
    players: Vec<ResponseRoomCheckPlayer>,
    room_status: String,
    owner_id: u32,
    leader_id: u32,
    round_counter: u8,
    round_total: u8,
    prompt_text: Option<String>,
    finishers: Option<Vec<ResponseRoomCheckFinisher>>
}

#[derive(Serialize, Debug)]
struct ResponseRoomCheckPlayer {
    player_id: u32,
    player_name: String,
    score: u8,
    is_finisher_ready: Option<bool>,
    is_next_round_ready: Option<bool>,
    last_check: u16
}

#[derive(Serialize, Debug)]
struct ResponseRoomCheckFinisher {
    player_name: String,
    finisher_text: String,
    is_winner: bool
}


#[derive(Deserialize, Debug)]
struct RequestGameStart {
    room_id: u32,
    player_id: u32
}


#[derive(Deserialize, Debug)]
struct RequestGameOptions {
    room_id: u32,
    player_id: u32
}

#[derive(Serialize, Debug)]
struct ResponseGameOptions {
    options: Vec<ResponseGameOptionsOption>
}

#[derive(Serialize, Debug)]
struct ResponseGameOptionsOption {
    option_id: u16,
    option_text: String
}


#[derive(Deserialize, Debug)]
struct RequestGamePick {
    room_id: u32,
    player_id: u32,
    option_id: u16
}
