use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufReader};
use std::env;
use std::option::Option;
use std::str;

use rand::Rng;
use rand::distributions::{Alphanumeric, DistString};
use rand::rngs::ThreadRng;

use serde::{Deserialize, Serialize};
use tiny_http::{Header, HeaderField, Method, Response, Server, StatusCode};


fn main() {
    let port = match env::var("PORT") {
        Ok(p) => p.parse::<u16>().unwrap(),
        Err(..) => 8000,
    };
 
    // Server (TPC bind) errors not handled for simplicity
    let host_port = format!("0.0.0.0:{}", port);

    //TODO: SSL
    println!("Starting server at {}.", host_port);
    let server = Server::http(host_port).unwrap();


    let mut rng = rand::thread_rng();
    let mut rooms: HashMap<u32, Room> = HashMap::new();
    let mut players: HashMap<u32, Player> = HashMap::new();
    let mut room_players: HashMap<u32, Vec<u32>> = HashMap::new();
    //TODO: is the mutable Game Context stuff thread-safe?
    let game_context = GameContext {
        rng: &mut rng,
        rooms: &mut rooms,
        players: &mut players,
        room_players: &mut room_players
    };


    //TODO: multiple workers
    for mut request in server.incoming_requests() {
        println!("received request! method: {:?}, url: {:?}", request.method(), request.url());
        // Headers are noisy:
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
                        //TODO: I know the code is reapeating A LOT, but got to learn how to use Rust's ownership before I can start doing refactoring things properly
                        // Could the HeaderField be a constant?
                        let content_type_header_field = HeaderField::from_bytes(b"Content-Type").unwrap();
                        let content_type_found = request.headers().iter().find(|&h| h.field == content_type_header_field);
                        if content_type_found.is_none() || content_type_found.unwrap().value != "application/json; charset=UTF-8" {
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
                                                let room_id: u32 = game_context.rng.gen();
                                                let room_code = Alphanumeric.sample_string(game_context.rng, 6);
                                                let room_code_for_response = room_code.clone();

                                                let room = Room{
                                                    id : room_id,
                                                    code : room_code,
                                                    room_status : RoomStatus::Waiting
                                                };
                                                game_context.rooms.insert(room_id, room);
                                                
                                                let player_id = game_context.rng.gen();
                                                let player = Player {
                                                    id : player_id,
                                                    name : trimmed_owner_name.to_string(),
                                                    is_owner : true,
                                                    is_leader : false,
                                                    position : 0,
                                                    score : 0
                                                };
                                                game_context.players.insert(player_id, player);

                                                game_context.room_players.insert(room_id, vec![player_id]);


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
                                                    println!("RoomJoin - Cant find room");

                                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                    request.respond(response).unwrap();
                                                } else {
                                                    let room_id = room_found.unwrap().id;

                                                    //TODO: Join player: check player name is not already in the Room
                                                    //TODO: Validate the room is in the context, otherwise 500
                                                    let players_in_room = game_context.room_players.get_mut(&room_id).unwrap();

                                                    
                                                    //TODO: Need thread safe id/code generator that doesn't repeat values...
                                                    let player_id = game_context.rng.gen();
                                                    let player = Player {
                                                        id : player_id,
                                                        name : trimmed_player_name.to_string(),
                                                        is_owner : false,
                                                        is_leader : false,
                                                        // Could this ever fail?
                                                        position : u8::try_from(players_in_room.len()).unwrap(),
                                                        score : 0
                                                    };
                                                    game_context.players.insert(player_id, player);

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
                        // Could the HeaderField be a constant?
                        let content_type_header_field = HeaderField::from_bytes(b"Content-Type").unwrap();
                        let content_type_found = request.headers().iter().find(|&h| h.field == content_type_header_field);
                        if content_type_found.is_none() || content_type_found.unwrap().value != "application/json; charset=UTF-8" {
                            println!("RoomCreate - Bad headers");

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
                                                    println!("RoomCheck - Room not found");

                                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                    request.respond(response).unwrap();
                                                } else {
                                                    //TODO: Validate the room is in the context, otherwise 500
                                                    let players_in_room = game_context.room_players.get(&room_id).unwrap();

                                                    //TODO: Check player belongs to room

                                                    let players_in_room_respose = players_in_room.iter().map(|room_player_id| {
                                                        //TODO: Validate the user is in the context, otherwise 500
                                                        let room_player = game_context.players.get(&room_player_id).unwrap();
                                                        ResponseRoomCheckPlayer::from(room_player) 
                                                    }).collect();
                                                    
                                                    let room_status = room_found.unwrap().room_status.to_string();


                                                    let response_room_create = ResponseRoomCheck { players: players_in_room_respose, room_status: room_status };
                                                    let serialized_response = serde_json::to_string(&response_room_create).unwrap();
                                                    let response_reader = BufReader::new(serialized_response.as_bytes());
                                                    let response = Response::new(StatusCode(201), headers, response_reader, Some(serialized_response.len()), None);
                                                    request.respond(response).unwrap();
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
                        //TODO: Join player: check owner request for start

                        
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
                                                    println!("GameStart - Room not found");
                                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                    request.respond(response).unwrap();
                                                } else {
                                                    //TODO: Check player belongs to room
                                                    //let players_in_room = game_context.room_players.get(room_id);

                                                    //TODO: Check there are enough players
                                                    
                                                    let room = room_found.unwrap();

                                                    match room.room_status {
                                                        RoomStatus::Waiting => {
                                                            room.room_status = RoomStatus::Playing;

                                                            let response = Response::new(StatusCode(204), headers, io::empty(), None, None);
                                                            request.respond(response).unwrap();
                                                        },
                                                        _ => {
                                                            println!("GameStart - Room not waiting");
                                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                                            request.respond(response).unwrap();
                                                        }
                                                    }
                                                }
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
                    _ => {
                        println!("Unknown Request");

                        let response = Response::new(StatusCode(500), headers, io::empty(), None, None);
                        request.respond(response).unwrap();
                    }
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
            "/game-status" => return Some(GameAction::GameStatus),
            "/game-pick" => return Some(GameAction::GamePick),
            _ => return None
        },
        Method::Options => return Some(GameAction::CorsOption),
        _ => return None
    };
}

// Should be useful later... I think
// let values = match map.entry(key) {
//     Entry::Occupied(o) => o.into_mut(),
//     Entry::Vacant(v) => v.insert(default),
// };



impl From<&Player> for ResponseRoomCheckPlayer {
    fn from(player: &Player) -> Self {
        Self {
            player_id : player.id,
            player_name : player.name.to_string(),
            is_owner : player.is_owner
        }
    }
}

impl fmt::Display for RoomStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RoomStatus::Waiting => write!(f, "WAITING"),
            RoomStatus::Playing => write!(f, "PLAYING")
        }
    }
}


struct GameContext<'a> {
    rng: &'a mut ThreadRng,
    rooms: &'a mut HashMap<u32, Room>,
    players: &'a mut HashMap<u32, Player>,
    room_players: &'a mut HashMap<u32, Vec<u32>>
}

enum GameAction {
    CorsOption,
    RoomCreate,
    RoomJoin,
    RoomCheck,
    GameStart,
    GameStatus,
    GamePick
}

struct Room {
    id: u32,
    code: String,
    room_status: RoomStatus
}

enum RoomStatus {
    Waiting,
    Playing
}

struct Player {
    id: u32,
    name: String,
    is_owner: bool,
    is_leader: bool,
    position: u8,
    score: u8
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
    room_status: String
}

#[derive(Serialize, Debug)]
struct ResponseRoomCheckPlayer {
    player_id: u32,
    player_name: String,
    is_owner: bool
}


#[derive(Deserialize, Debug)]
struct RequestGameStart {
    room_id: u32,
    player_id: u32
}
