use std::collections::HashMap;
use std::io::{self, BufReader};
use std::env;
use std::option::Option;
use std::str;

use rand::Rng;
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
        println!("received request! method: {:?}, url: {:?}, headers: {:?}",
            request.method(),
            request.url(),
            request.headers()
        );

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
                        // Could the HeaderField be a constant?
                        let content_type_header_field = HeaderField::from_bytes(b"Content-Type").unwrap();
                        let content_type_found = request.headers().iter().find(|&h| h.field == content_type_header_field);
                        if content_type_found.is_none() || content_type_found.unwrap().value != "application/json; charset=UTF-8" {
                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                            request.respond(response).unwrap();
                        } else {

                            let mut content = String::new();
                            let reader = request.as_reader();
                            match reader.read_to_string(&mut content) {
                                Ok(_) => {

                                    match serde_json::from_str::<RequestRoomCreate>(&mut content) {
                                        Ok(deserialized_request) => {

                                            //TODO: Need thread safe id generator that doesn't repeat values...
                                            let room_id: u32 = game_context.rng.gen();
                                            let room_code = "abc".to_string(); //TODO: later generate random words
                                            let room_code2 = room_code.clone(); //TODO: later generate random words

                                            let room = Room{
                                                id : room_id,
                                                code : room_code,
                                                room_status : RoomStatus::Waiting
                                            };
                                            game_context.rooms.insert(room_id, room);
                                            
                                            let player_id = game_context.rng.gen();
                                            let player = Player {
                                                id : player_id,
                                                name : deserialized_request.name,
                                                owner : true,
                                                leader : false,
                                                position : 0,
                                                score : 0
                                            };
                                            game_context.players.insert(player_id, player);

                                            game_context.room_players.insert(room_id, vec![player_id]);

                                            println!("{}", game_context.room_players.len());

                                            let response = ResponseRoomCreate { room_id: room_id, room_code: room_code2, player_id: player_id };

                                            let serialized_response = serde_json::to_string(&response).unwrap();

                                            // let response = Response::from_string(serialized_response);
                                            // response.with_status_code(StatusCode(201));
                                            // response.add_header(access_control_allow_headers);
                                            // response.add_header(access_control_allow_origin_header);
                                            // response.add_header(access_control_allow_methods);
                                            // response.add_header(access_control_allow_max_age);
                                            let response_reader = BufReader::new(serialized_response.as_bytes());
                                            let response = Response::new(StatusCode(201), headers, response_reader, Some(serialized_response.len()), None);
                                            request.respond(response).unwrap();

                                        },
                                        Err(_) => {
                                            let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                            request.respond(response).unwrap();
                                        }
                                    };

                                },
                                Err(_) => {
                                    let response = Response::new(StatusCode(400), headers, io::empty(), None, None);
                                    request.respond(response).unwrap();
                                }
                            }
                        }
                    },
                    //TODO: Join player: check player name is not already in the Room
                    _ => {
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
        Method::Get => match url {
            "/room-check" => return Some(GameAction::RoomCheck),
            "/game-status" => return Some(GameAction::GameStatus),
            _ => return None
        },
        Method::Post => match url {
            "/room-create" => return Some(GameAction::RoomCreate),
            "/room-join" => return Some(GameAction::RoomJoin),
            "/game-start" => return Some(GameAction::GameStart),
            "/game-pick" => return Some(GameAction::GamePick),
            _ => return None
        },
        Method::Options => return Some(GameAction::CorsOption),
        _ => return None
    };
}

// let values = match map.entry(key) {
//     Entry::Occupied(o) => o.into_mut(),
//     Entry::Vacant(v) => v.insert(default),
// };

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
    owner: bool,
    leader: bool,
    position: u8,
    score: u8
}


#[derive(Deserialize, Debug)]
struct RequestRoomCreate {
    name: String
}

#[derive(Serialize, Debug)]
struct ResponseRoomCreate {
    room_id: u32,
    room_code: String,
    player_id: u32
}
