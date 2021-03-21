use serde::{Serialize, Deserialize};
use rocket::{Request, State};
use crate::web::{SessionManager, SessionKeystore};
use rocket::request::{FromRequest, Outcome};
use rocket::http::Status;
use crate::storage::Storage;

#[derive(Serialize, Deserialize)]
pub struct User {
    pub username: String,
    pub photo: Option<String>,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for User {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let keystore = request.guard::<State<SessionKeystore>>().await.unwrap();
        let cookies = request.cookies();
        let storage = request.guard::<State<Storage>>().await.unwrap();
        let mut sm = SessionManager::new(cookies, keystore, storage);
        if sm.is_authorized() {
            Outcome::Success(sm.get_user().unwrap())
        } else {
            Outcome::Failure((Status::Unauthorized, ()))
        }
    }
}
