use crate::account::Session;
use ic_plugin_api::{IcPinnedConnection, IcPinnedConnections};
use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_void;

pub const SOURCE_ID: &str = "nodeinnet.account";
pub const ENTRY_ID: &str = "account";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    pub svg: &'static str,
}

pub fn entries(session: &Session) -> Vec<Entry> {
    if !session.has_account() {
        return Vec::new();
    }
    let login = session.account.login.trim();
    vec![Entry {
        title: if login.is_empty() {
            "Node In Net".to_string()
        } else {
            login.to_string()
        },
        svg: if crate::account::is_connected(session) {
            crate::CONNECTED
        } else {
            crate::DISCONNECTED
        },
    }]
}

thread_local! {
    static HELD: RefCell<Vec<[CString; 4]>> = const { RefCell::new(Vec::new()) };
    static SHOWN: RefCell<Vec<IcPinnedConnection>> = const { RefCell::new(Vec::new()) };
}

pub extern "C" fn rows(_user_data: *mut c_void) -> IcPinnedConnections {
    let held: Vec<[CString; 4]> = entries(&crate::snapshot())
        .into_iter()
        .filter_map(|entry| {
            Some([
                CString::new(ENTRY_ID).ok()?,
                CString::new(entry.title).ok()?,
                CString::new(entry.svg).ok()?,
                CString::new(crate::view::VIEW_ID).ok()?,
            ])
        })
        .collect();
    HELD.with(|slot| *slot.borrow_mut() = held);
    HELD.with(|slot| {
        let held = slot.borrow();
        SHOWN.with(|shown| {
            let mut shown = shown.borrow_mut();
            *shown = held
                .iter()
                .map(|[id, title, svg, view]| IcPinnedConnection {
                    id: id.as_ptr(),
                    title: title.as_ptr(),
                    svg: svg.as_ptr(),
                    view: view.as_ptr(),
                })
                .collect();
            IcPinnedConnections {
                count: shown.len() as u32,
                rows: shown.as_ptr(),
                ..IcPinnedConnections::EMPTY
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Phase;

    #[test]
    fn without_an_account_there_is_no_entry() {
        assert!(entries(&Session::signed_out()).is_empty());
        let signing_in = Session {
            phase: Phase::Working("signing in"),
            ..Session::default()
        };
        assert!(entries(&signing_in).is_empty());
    }

    #[test]
    fn the_entry_is_named_after_the_account_and_drawn_as_the_header_is() {
        let signed = Session::resumed("r", "alice", false);
        assert_eq!(
            entries(&signed),
            vec![Entry {
                title: "alice".to_string(),
                svg: crate::CONNECTED
            }]
        );
        for phase in [Phase::Offline, Phase::Working("checking")] {
            let away = Session {
                phase,
                ..signed.clone()
            };
            assert_eq!(entries(&away)[0].svg, crate::DISCONNECTED);
            assert_eq!(entries(&away)[0].title, "alice");
        }
    }
}
