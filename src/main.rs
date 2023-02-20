#[macro_use]
extern crate rocket;
use std::{collections::HashMap, sync::RwLock};

use rocket::{
    fairing::{Fairing, Info, Kind},
    http::Header,
    response::stream::{Event, EventStream},
    serde::{json::Json, Deserialize, Serialize},
    tokio::select,
    tokio::sync::broadcast::{channel, error::RecvError, Sender},
    Shutdown, State, {Request, Response},
};

#[derive(Debug, Clone, FromForm, Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
struct Message {
    #[field(validate = len(..20))]
    pub sender: String,
    #[field(validate = len(..30))]
    pub room: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, FromForm, Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
struct RoomWithMessages {
    name: String,
    messages: Vec<Message>,
}

pub struct CORS;

pub struct RoomsMessagesCache {
    entries: RwLock<HashMap<String, Vec<Message>>>,
}

impl RoomsMessagesCache {
    fn new() -> RoomsMessagesCache {
        let messages_cache: RoomsMessagesCache = RoomsMessagesCache {
            entries: RwLock::new(HashMap::new()),
        };

        return messages_cache;
    }

    fn get(&self, room: &str) -> Option<Vec<Message>> {
        let messages = self.entries.read().unwrap();
        return messages.get(room).cloned();
    }

    fn set(&self, message: Message) {
        let mut cache = self.entries.write().unwrap();
        let messages = cache.get_mut(&message.room);
        match messages {
            None => {
                cache.insert(message.room.clone(), vec![message]);
            }
            Some(room_messages) => {
                room_messages.push(message);
                if room_messages.len() > 10 {
                    room_messages.remove(0);
                }
            }
        };
    }
}

#[rocket::async_trait]
impl Fairing for CORS {
    fn info(&self) -> Info {
        Info {
            name: "Add CORS headers to responses",
            kind: Kind::Response,
        }
    }

    async fn on_response<'r>(&self, _request: &'r Request<'_>, response: &mut Response<'r>) {
        response.set_header(Header::new("Access-Control-Allow-Origin", "*"));
        response.set_header(Header::new(
            "Access-Control-Allow-Methods",
            "POST, GET, PATCH, OPTIONS",
        ));
        response.set_header(Header::new("Access-Control-Allow-Headers", "*"));
        response.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
    }
}

#[options("/<_..>")]
fn all_options() {}

#[post("/message", format = "json", data = "<message>")]
fn post_message(
    message: Json<Message>,
    queue: &State<Sender<Message>>,
    cache: &State<RoomsMessagesCache>,
) {
    cache.set(Message {
        content: message.content.clone(),
        room: message.room.clone(),
        sender: message.sender.clone(),
        created_at: message.created_at.clone(),
    });

    let _res = queue.send(message.into_inner());
}

#[get("/messages?<room>", format = "json")]
fn get_messages(room: String, cache: &State<RoomsMessagesCache>) -> Json<RoomWithMessages> {
    let messages = cache.get(&room);
    match messages {
        None => {
            return Json(RoomWithMessages {
                name: room.to_string(),
                messages: vec![],
            })
        }
        Some(messages) => {
            return Json(RoomWithMessages {
                name: room.to_string(),
                messages: messages.clone(),
            });
        }
    }
}

#[get("/events")]
async fn events(queue: &State<Sender<Message>>, mut end: Shutdown) -> EventStream![] {
    let mut rx = queue.subscribe();

    return EventStream! {
        loop {
            let msg = select! {
                msg = rx.recv() => match msg {
                    Ok(msg) => msg,
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(_)) => continue,
                },
                _ = &mut end => break,
            };
            yield Event::json(&msg);

        }
    };
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .attach(CORS)
        .manage(channel::<Message>(1024).0)
        .manage(RoomsMessagesCache::new())
        .mount(
            "/",
            routes![post_message, get_messages, events, all_options],
        )
}
