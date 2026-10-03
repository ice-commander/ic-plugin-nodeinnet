pub use crate::auth::Failure;
use nodeinnet_protocol::TurnRegion;
use serde_json::Value;

pub const DEFAULT_API: &str = "https://node.in.net";

pub fn endpoint_from(set: Option<&str>) -> String {
    match set.map(str::trim).filter(|url| !url.is_empty()) {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => DEFAULT_API.to_string(),
    }
}

pub fn endpoint() -> String {
    endpoint_from(std::env::var("NODEINNET_API").ok().as_deref())
}

pub trait Directory: Send + Sync {
    fn sign_in(&self, login: &str, password: &str) -> Result<Value, Failure>;
    fn check(&self, token: &str) -> Result<Value, Failure>;
    fn sign_out(&self, token: &str) -> Result<(), Failure>;
}

pub struct Net {
    base: String,
}

impl Net {
    pub fn new() -> Self {
        Net { base: endpoint() }
    }

    pub fn at(base: &str) -> Self {
        Net {
            base: base.trim_end_matches('/').to_string(),
        }
    }
}

impl Default for Net {
    fn default() -> Self {
        Net::new()
    }
}

fn awaited<T>(work: impl std::future::Future<Output = Result<T, Failure>>) -> Result<T, Failure> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|why| Failure::Unanswered(format!("no runtime: {why}")))?
        .block_on(work)
}

fn as_value(answer: impl serde::Serialize) -> Result<Value, Failure> {
    serde_json::to_value(answer).map_err(|why| Failure::Unanswered(why.to_string()))
}

impl Directory for Net {
    fn sign_in(&self, login: &str, password: &str) -> Result<Value, Failure> {
        as_value(awaited(crate::auth::login(
            &self.base,
            login,
            password,
            TurnRegion::Auto,
        ))?)
    }

    fn check(&self, token: &str) -> Result<Value, Failure> {
        as_value(awaited(crate::auth::refresh_access_token(
            &self.base,
            token,
            TurnRegion::Auto,
        ))?)
    }

    fn sign_out(&self, token: &str) -> Result<(), Failure> {
        awaited(crate::auth::logoff(&self.base, token, None))
    }
}

#[cfg(test)]
pub struct Fake {
    pub answer: std::sync::Mutex<Result<Value, Failure>>,
    pub asked: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl Fake {
    pub fn saying(answer: Result<Value, Failure>) -> Self {
        Fake {
            answer: std::sync::Mutex::new(answer),
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn note(&self, what: &str) -> Result<Value, Failure> {
        self.asked.lock().unwrap().push(what.to_string());
        self.answer.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Directory for Fake {
    fn sign_in(&self, login: &str, password: &str) -> Result<Value, Failure> {
        self.note(&format!("sign_in {login}:{password}"))
    }

    fn check(&self, token: &str) -> Result<Value, Failure> {
        self.note(&format!("check {token}"))
    }

    fn sign_out(&self, token: &str) -> Result<(), Failure> {
        self.note(&format!("sign_out {token}")).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_reported_in_the_servers_own_words() {
        let answered = serde_json::json!({ "msg": "Invalid login or password." });
        assert_eq!(
            crate::auth::refused(401, Some(&answered), "Invalid login or password"),
            "Invalid login or password."
        );
    }

    #[test]
    fn a_refusal_that_explains_nothing_falls_back_to_the_status() {
        let expected = "Server returned error status (503)";
        assert_eq!(
            crate::auth::refused(503, None, "Server returned error status"),
            expected
        );
        for body in [
            serde_json::json!({}),
            serde_json::json!({ "msg": "   " }),
            serde_json::json!({ "msg": 401 }),
            serde_json::json!("not an object"),
        ] {
            assert_eq!(
                crate::auth::refused(503, Some(&body), "Server returned error status"),
                expected,
                "{body}"
            );
        }
    }

    #[test]
    fn a_server_nobody_answers_at_is_not_mistaken_for_a_refusal() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let address = closed.local_addr().expect("an address");
        drop(closed);
        let net = Net::at(&format!("http://{address}"));
        for failure in [
            net.check("r").expect_err("nobody is there"),
            net.sign_in("alice", "x").expect_err("nobody is there"),
        ] {
            assert!(
                matches!(failure, Failure::Unanswered(_)),
                "{failure:?} would sign the account out"
            );
        }
    }

    #[test]
    fn a_server_that_says_no_is_a_refusal() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let address = listener.local_addr().expect("an address");
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let Ok(mut stream) = stream else { continue };
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let body = r#"{"msg":"Invalid login or password."}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let net = Net::at(&format!("http://{address}"));
        assert_eq!(
            net.check("r"),
            Err(Failure::Refused(
                401,
                "Invalid login or password.".to_string()
            ))
        );
        assert_eq!(
            net.sign_in("alice", "x"),
            Err(Failure::Refused(
                401,
                "Invalid login or password.".to_string()
            ))
        );
    }

    #[test]
    fn the_endpoint_falls_back_to_the_published_one() {
        assert_eq!(endpoint_from(None), DEFAULT_API);
        assert_eq!(endpoint_from(Some("   ")), DEFAULT_API);
        assert_eq!(
            endpoint_from(Some(" http://127.0.0.1:8030/ ")),
            "http://127.0.0.1:8030"
        );
    }

    #[test]
    fn a_trailing_slash_is_not_carried_into_the_path() {
        let net = Net::at("https://example.org/");
        assert_eq!(net.base, "https://example.org");
    }
}
