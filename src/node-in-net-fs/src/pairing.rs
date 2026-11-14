use crate::i18n::{tr, trf};
use crate::notify;
use client_core::{LanCmd, PairDecision, PairOutcome, PairPrompt, PairReply, RefuseCode};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub const PAIR_VIEW: &str = "nodeinnet.pair";
pub const PAIRING_VIEW: &str = "nodeinnet.pairing";
pub const ALLOW_AFTER: Duration = Duration::from_secs(1);

const ALLOW: &str = "allow:";
const DENY: &str = "deny:";
const BLOCK: &str = "block:";
const CONFIRM: &str = "confirm:";
const CANCEL: &str = "cancel:";
const CLOSE: &str = "close";

struct Waiting {
    prompt: PairPrompt,
    reply: PairReply,
    shown_at: Option<Instant>,
    row_shown_at: Option<Instant>,
}

impl Waiting {
    fn shown(&self) -> bool {
        !self.prompt.quiet
    }
}

#[derive(Default)]
struct Requests {
    queue: VecDeque<Waiting>,
    on_screen: Option<String>,
}

#[derive(Clone)]
enum Phase {
    Comparing { shown_at: Option<Instant> },
    Waiting,
    Done(PairOutcome),
}

#[derive(Clone)]
struct Outgoing {
    id: String,
    code: String,
    phase: Phase,
}

struct Kept {
    outcome: PairOutcome,
    at: Instant,
    told: bool,
}

#[derive(Default)]
struct Asking {
    current: Option<Outgoing>,
    last: HashMap<String, Kept>,
}

fn requests() -> MutexGuard<'static, Requests> {
    static HELD: OnceLock<Mutex<Requests>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Requests::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn asking() -> MutexGuard<'static, Asking> {
    static HELD: OnceLock<Mutex<Asking>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Asking::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn ready(shown_at: Option<Instant>, now: Instant) -> bool {
    shown_at.is_some_and(|at| now.saturating_duration_since(at) >= ALLOW_AFTER)
}

/// A loud request raises the window again, so the one already on it waits another second.
pub fn incoming(prompt: PairPrompt, reply: PairReply) {
    let quiet = prompt.quiet;
    let mut held = requests();
    if !quiet {
        if let Some(head) = held.queue.iter_mut().find(|waiting| waiting.shown()) {
            head.shown_at = None;
        }
    }
    held.queue.push_back(Waiting {
        prompt,
        reply,
        shown_at: None,
        row_shown_at: None,
    });
    drop(held);
    if !quiet {
        notify::mark(notify::OPEN_PAIR | notify::PAIR);
    }
}

pub fn prompt_closed(request: &str) {
    let gone = {
        let mut held = requests();
        let at = held
            .queue
            .iter()
            .position(|waiting| waiting.prompt.request == request);
        at.and_then(|at| held.queue.remove(at))
    };
    if gone.is_some() {
        notify::mark(notify::PAIR | notify::SETTINGS);
    }
}

pub fn comparing(id: String, code: String) {
    asking().current = Some(Outgoing {
        id,
        code,
        phase: Phase::Comparing { shown_at: None },
    });
    notify::mark(notify::OPEN_PAIRING | notify::PAIRING);
}

pub fn done(id: String, outcome: PairOutcome) {
    let shown = {
        let mut held = asking();
        let shown = match held.current.as_mut() {
            Some(outgoing) if outgoing.id == id => {
                outgoing.phase = Phase::Done(outcome.clone());
                true
            }
            _ => false,
        };
        held.last.insert(
            id,
            Kept {
                outcome,
                at: Instant::now(),
                told: shown,
            },
        );
        shown
    };
    if shown {
        notify::mark(notify::PAIRING);
    }
}

pub fn last_outcome(id: &str) -> Option<(PairOutcome, Instant)> {
    asking()
        .last
        .get(id)
        .map(|kept| (kept.outcome.clone(), kept.at))
}

/// For a device not paired yet: what to say instead of comparing codes, and whether to ask it again.
pub fn next_try(id: &str) -> (Option<String>, bool) {
    let outcome = {
        let mut held = asking();
        let Some(kept) = held.last.get_mut(id) else {
            return (None, true);
        };
        if let PairOutcome::Refused {
            retry_after: Some(wait),
            ..
        } = kept.outcome
        {
            if kept.at.elapsed() >= wait {
                return (None, true);
            }
            let standing = kept.outcome.clone();
            drop(held);
            return (Some(outcome_text(&standing, &named(id))), false);
        }
        if kept.told || kept.outcome == PairOutcome::Allowed {
            return (None, true);
        }
        kept.told = true;
        kept.outcome.clone()
    };
    (Some(outcome_text(&outcome, &named(id))), true)
}

fn named(id: &str) -> String {
    crate::lan::snapshot()
        .state
        .and_then(|state| state.devices.into_iter().find(|device| device.id == id))
        .map(|device| device.name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| p2p_node::lan::id::fingerprint(id))
}

pub fn forget() {
    let dropped = {
        let mut held = requests();
        held.on_screen = None;
        std::mem::take(&mut held.queue)
    };
    let asked = asking().current.take().is_some();
    if !dropped.is_empty() || asked {
        notify::mark(notify::PAIR | notify::PAIRING | notify::SETTINGS);
    }
}

pub(crate) fn anything_to_show() -> bool {
    requests().queue.iter().any(Waiting::shown)
}

pub(crate) fn outgoing_to_show() -> bool {
    asking().current.is_some()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitingRow {
    pub request: String,
    pub name: String,
    pub os: String,
    pub code: String,
    pub fingerprint: String,
    pub known: bool,
    pub name_clash: bool,
    pub quiet: bool,
    pub allow_ready: bool,
}

pub fn waiting_rows() -> Vec<WaitingRow> {
    let now = Instant::now();
    let (rows, earliest) = {
        let mut held = requests();
        let mut earliest: Option<Instant> = None;
        let rows = held
            .queue
            .iter_mut()
            .map(|waiting| {
                let shown_at = *waiting.row_shown_at.get_or_insert(now);
                let allow_ready = ready(Some(shown_at), now);
                if !allow_ready {
                    earliest = Some(earliest.map_or(shown_at, |held| held.min(shown_at)));
                }
                WaitingRow {
                    request: waiting.prompt.request.clone(),
                    name: waiting.prompt.name.clone(),
                    os: waiting.prompt.os.clone(),
                    code: waiting.prompt.code.clone(),
                    fingerprint: waiting.prompt.fingerprint.clone(),
                    known: waiting.prompt.known,
                    name_clash: waiting.prompt.name_clash,
                    quiet: waiting.prompt.quiet,
                    allow_ready,
                }
            })
            .collect();
        (rows, earliest)
    };
    if let Some(at) = earliest {
        notify::invalidate_at(crate::view::VIEW_ID, at + ALLOW_AFTER);
    }
    rows
}

pub fn answer_waiting(request: &str, decision: PairDecision) -> bool {
    let answered = {
        let mut held = requests();
        let Some(at) = held
            .queue
            .iter()
            .position(|waiting| waiting.prompt.request == request)
        else {
            return false;
        };
        let allowing = matches!(decision, PairDecision::Allow { .. });
        if allowing && !ready(held.queue[at].row_shown_at, Instant::now()) {
            return false;
        }
        held.queue.remove(at)
    };
    let Some(answered) = answered else {
        return false;
    };
    answered.reply.answer(decision);
    notify::mark(notify::PAIR | notify::SETTINGS);
    true
}

fn said(key: &str, en: &str) -> Value {
    json!({ "tr": key, "en": en })
}

fn button(id: &str, role: Option<&str>, label: Value) -> Value {
    let mut node = json!({ "t": "button", "id": id, "label": label,
                           "intent": { "do": "emit", "node": id } });
    if let Some(role) = role {
        node["role"] = json!(role);
    }
    node
}

fn closing() -> Value {
    json!({ "t": "row", "children": [
        button(CLOSE, None, said("nodeinnet.close", "Close"))
    ]})
}

fn dialog(kind: &str, data: Value, fields: Value, height: u32, children: Vec<Value>) -> Value {
    json!({
        "schema": 1,
        "kind": kind,
        "data": data,
        "fields": fields,
        "form": {
            "t": "view",
            "surface": "dialog",
            "width": 460,
            "height": height,
            "padding": 18,
            "spacing": 10,
            "children": children
        }
    })
}

fn request_document(prompt: &PairPrompt, allow_ready: bool, others: usize) -> Value {
    let allow = format!("{ALLOW}{}", prompt.request);
    let deny = format!("{DENY}{}", prompt.request);
    let block = format!("{BLOCK}{}", prompt.request);
    let count = others.to_string();
    let more = if others == 0 {
        String::new()
    } else {
        trf("nodeinnet.pair_more_waiting", &[("count", &count)])
    };
    let mut allowing = button(
        &allow,
        Some("primary"),
        said("nodeinnet.pair_allow", "Codes match — Allow"),
    );
    allowing["sensitive"] = json!({ "truthy": "data.allow_ready" });
    dialog(
        PAIR_VIEW,
        json!({
            "question": trf("nodeinnet.pair_question",
                            &[("name", &prompt.name), ("os", &prompt.os)]),
            "code": prompt.code,
            "fingerprint": trf("nodeinnet.pair_fingerprint",
                               &[("fingerprint", &prompt.fingerprint)]),
            "known": prompt.known,
            "name_clash": prompt.name_clash,
            "more": more,
            "allow_ready": allow_ready,
        }),
        json!([{ "bind": "remember", "type": "bool", "default": true }]),
        420,
        vec![
            json!({ "t": "text", "id": "question", "role": "title2", "wrap": true,
                    "text": "{data.question}" }),
            json!({ "t": "text", "id": "compare", "role": "dim", "wrap": true,
                    "text": said("nodeinnet.pair_compare",
                                 "Allow only if the other device shows the same code") }),
            json!({ "t": "text", "id": "code", "role": "title1", "text": "{data.code}" }),
            json!({ "t": "text", "id": "fingerprint", "role": "dim",
                    "text": "{data.fingerprint}" }),
            json!({ "t": "text", "id": "known", "wrap": true,
                    "text": said("nodeinnet.pair_known", "This device is already in your list"),
                    "visible": { "truthy": "data.known" } }),
            json!({ "t": "text", "id": "name_clash", "role": "error", "wrap": true,
                    "text": said("nodeinnet.pair_name_clash",
                                 "Another device on this network has the same name"),
                    "visible": { "truthy": "data.name_clash" } }),
            json!({ "t": "switch", "id": "remember", "bind": "remember", "chrome": "row",
                    "title": said("nodeinnet.pair_remember", "Remember this device") }),
            json!({ "t": "text", "id": "more", "role": "dim", "text": "{data.more}",
                    "visible": { "not": { "empty": "data.more" } } }),
            json!({ "t": "row", "spacing": 8, "children": [
                button(&deny, None, said("nodeinnet.pair_deny", "Deny")),
                button(&block, Some("destructive"), said("nodeinnet.pair_block", "Block")),
                allowing
            ]}),
        ],
    )
}

fn expired_document() -> Value {
    dialog(
        PAIR_VIEW,
        json!({}),
        json!([]),
        200,
        vec![
            json!({ "t": "text", "id": "expired", "role": "heading", "wrap": true,
                    "text": said("nodeinnet.pair_expired", "This request has expired") }),
            closing(),
        ],
    )
}

pub fn pair_document() -> Value {
    let now = Instant::now();
    let shown = {
        let mut held = requests();
        let others = held
            .queue
            .iter()
            .filter(|waiting| waiting.shown())
            .count()
            .saturating_sub(1);
        let shown = held
            .queue
            .iter_mut()
            .find(|waiting| waiting.shown())
            .map(|waiting| {
                let shown_at = *waiting.shown_at.get_or_insert(now);
                (waiting.prompt.clone(), shown_at, others)
            });
        held.on_screen = shown.as_ref().map(|(prompt, ..)| prompt.request.clone());
        shown
    };
    let Some((prompt, shown_at, others)) = shown else {
        return expired_document();
    };
    let allow_ready = ready(Some(shown_at), now);
    if !allow_ready {
        notify::invalidate_at(PAIR_VIEW, shown_at + ALLOW_AFTER);
    }
    request_document(&prompt, allow_ready, others)
}

fn decided(node: &str, remember: bool) -> Option<(PairDecision, &str)> {
    if let Some(request) = node.strip_prefix(ALLOW) {
        return Some((PairDecision::Allow { remember }, request));
    }
    if let Some(request) = node.strip_prefix(DENY) {
        return Some((PairDecision::Deny, request));
    }
    node.strip_prefix(BLOCK)
        .map(|request| (PairDecision::Block, request))
}

pub fn pair_reply(event: &Value) -> Value {
    if event["type"].as_str() != Some(ic_plugin_api::IC_EVENT_ACTIVATE) {
        return json!({});
    }
    let node = event["node"].as_str().unwrap_or_default();
    if node == CLOSE {
        return json!({ "close": true });
    }
    let remember = event["values"]["remember"].as_bool().unwrap_or(true);
    let Some((decision, request)) = decided(node, remember) else {
        return json!({});
    };
    let now = Instant::now();
    let (answered, waiting) = {
        let mut held = requests();
        let at = held
            .queue
            .iter()
            .position(|waiting| waiting.shown() && waiting.prompt.request == request);
        if let Some(at) = at {
            let allowing = matches!(decision, PairDecision::Allow { .. });
            if allowing && !ready(held.queue[at].shown_at, now) {
                return json!({});
            }
        }
        let answered = at.and_then(|at| held.queue.remove(at));
        (answered, held.queue.iter().any(Waiting::shown))
    };
    let Some(answered) = answered else {
        return json!({ "redescribe": true });
    };
    answered.reply.answer(decision);
    notify::mark(notify::SETTINGS);
    if waiting {
        json!({ "redescribe": true, "put": { "remember": true } })
    } else {
        json!({ "close": true })
    }
}

/// Only the request this window showed goes unanswered: a newer one may have been queued since.
pub fn pair_closed() {
    let (dropped, waiting) = {
        let mut held = requests();
        let showed = held.on_screen.take();
        let at = showed.and_then(|request| {
            held.queue
                .iter()
                .position(|waiting| waiting.prompt.request == request)
        });
        let dropped = at.and_then(|at| held.queue.remove(at));
        (dropped, held.queue.iter().any(Waiting::shown))
    };
    if dropped.is_some() {
        notify::mark(notify::SETTINGS);
    }
    if waiting {
        notify::mark(notify::OPEN_PAIR);
    }
}

fn outcome_text(outcome: &PairOutcome, name: &str) -> String {
    let named = [("name", name)];
    match outcome {
        PairOutcome::Allowed => tr("nodeinnet.pairing_paired"),
        PairOutcome::Denied
        | PairOutcome::Refused {
            code: RefuseCode::Blocked,
            ..
        } => tr("nodeinnet.pairing_declined"),
        PairOutcome::Expired => tr("nodeinnet.pairing_expired"),
        PairOutcome::Cancelled => tr("nodeinnet.pairing_cancelled"),
        PairOutcome::NotShown => trf("nodeinnet.pairing_not_shown", &named),
        PairOutcome::Refused {
            code: RefuseCode::PairingClosed,
            ..
        } => trf("nodeinnet.pairing_closed", &named),
        PairOutcome::Refused {
            code: RefuseCode::NoName,
            ..
        } => tr("nodeinnet.pairing_no_name"),
        PairOutcome::Refused { .. } => trf("nodeinnet.pairing_busy", &named),
        PairOutcome::Failed(reason) => trf("nodeinnet.pairing_failed", &[("reason", reason)]),
    }
}

fn nothing_document() -> Value {
    dialog(
        PAIRING_VIEW,
        json!({}),
        json!([]),
        200,
        vec![
            json!({ "t": "text", "id": "nothing", "role": "heading", "wrap": true,
                    "text": said("nodeinnet.pairing_none", "No pairing in progress") }),
            closing(),
        ],
    )
}

fn outgoing_document(outgoing: &Outgoing, now: Instant) -> Value {
    let device = crate::lan::snapshot().state.and_then(|state| {
        state
            .devices
            .into_iter()
            .find(|device| device.id == outgoing.id)
    });
    let fingerprint = p2p_node::lan::id::fingerprint(&outgoing.id);
    let name = device
        .as_ref()
        .map(|device| device.name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| fingerprint.clone());
    let named = [("name", name.as_str())];
    let cancel = format!("{CANCEL}{}", outgoing.id);
    let confirm = format!("{CONFIRM}{}", outgoing.id);
    let (said_now, confirm_ready) = match &outgoing.phase {
        Phase::Comparing { shown_at } => (
            trf("nodeinnet.pairing_check", &named),
            ready(*shown_at, now),
        ),
        Phase::Waiting => (trf("nodeinnet.pairing_waiting", &named), false),
        Phase::Done(outcome) => (outcome_text(outcome, &name), false),
    };
    let data = json!({
        "said": said_now,
        "code": outgoing.code,
        "fingerprint": trf("nodeinnet.pair_fingerprint", &[("fingerprint", &fingerprint)]),
        "name_clash": device.as_ref().is_some_and(|device| device.name_clash),
        "confirm_ready": confirm_ready,
    });
    let telling = json!({ "t": "text", "id": "said", "role": "title2", "wrap": true,
                          "text": "{data.said}" });
    let code = json!({ "t": "text", "id": "code", "role": "title1", "text": "{data.code}" });
    let fingerprinted = json!({ "t": "text", "id": "fingerprint", "role": "dim",
                                "text": "{data.fingerprint}" });
    let cancelling = button(&cancel, None, said("nodeinnet.pairing_cancel", "Cancel"));
    match &outgoing.phase {
        Phase::Comparing { .. } => {
            let mut confirming = button(
                &confirm,
                Some("primary"),
                said("nodeinnet.pairing_codes_match", "Codes match"),
            );
            confirming["sensitive"] = json!({ "truthy": "data.confirm_ready" });
            dialog(
                PAIRING_VIEW,
                data,
                json!([]),
                360,
                vec![
                    telling,
                    code,
                    fingerprinted,
                    json!({ "t": "text", "id": "name_clash", "role": "error", "wrap": true,
                            "text": said("nodeinnet.pair_name_clash",
                                         "Another device on this network has the same name"),
                            "visible": { "truthy": "data.name_clash" } }),
                    json!({ "t": "row", "spacing": 8, "children": [cancelling, confirming] }),
                ],
            )
        }
        Phase::Waiting => dialog(
            PAIRING_VIEW,
            data,
            json!([]),
            300,
            vec![
                telling,
                code,
                fingerprinted,
                json!({ "t": "row", "children": [cancelling] }),
            ],
        ),
        Phase::Done(_) => dialog(
            PAIRING_VIEW,
            data,
            json!([]),
            220,
            vec![telling, fingerprinted, closing()],
        ),
    }
}

pub fn pairing_document() -> Value {
    let now = Instant::now();
    let shown = {
        let mut held = asking();
        if let Some(Outgoing {
            phase: Phase::Comparing { shown_at },
            ..
        }) = held.current.as_mut()
        {
            shown_at.get_or_insert(now);
        }
        held.current.clone()
    };
    let Some(outgoing) = shown else {
        return nothing_document();
    };
    if let Phase::Comparing { shown_at: Some(at) } = outgoing.phase {
        if !ready(Some(at), now) {
            notify::invalidate_at(PAIRING_VIEW, at + ALLOW_AFTER);
        }
    }
    outgoing_document(&outgoing, now)
}

pub fn pairing_reply(event: &Value) -> Value {
    if event["type"].as_str() != Some(ic_plugin_api::IC_EVENT_ACTIVATE) {
        return json!({});
    }
    let node = event["node"].as_str().unwrap_or_default();
    if node == CLOSE {
        return json!({ "close": true });
    }
    let now = Instant::now();
    if let Some(id) = node.strip_prefix(CONFIRM) {
        let code = {
            let mut held = asking();
            match held.current.as_mut() {
                Some(outgoing) if outgoing.id == id => match outgoing.phase {
                    Phase::Comparing { shown_at } if ready(shown_at, now) => {
                        outgoing.phase = Phase::Waiting;
                        Some(outgoing.code.clone())
                    }
                    Phase::Comparing { .. } => return json!({}),
                    _ => None,
                },
                _ => None,
            }
        };
        if let Some(code) = code {
            crate::net::lan(LanCmd::ConfirmPair(id.to_string(), code));
        }
        return json!({ "redescribe": true });
    }
    if let Some(id) = node.strip_prefix(CANCEL) {
        let asked = cancelled_if(|outgoing| {
            outgoing.id == id && matches!(outgoing.phase, Phase::Comparing { .. } | Phase::Waiting)
        });
        if let Some(id) = asked {
            crate::net::lan(LanCmd::CancelPair(id));
        }
        return json!({ "close": true });
    }
    json!({})
}

fn cancelled_if(wanted: impl Fn(&Outgoing) -> bool) -> Option<String> {
    let mut held = asking();
    let outgoing = held.current.as_mut().filter(|outgoing| wanted(outgoing))?;
    outgoing.phase = Phase::Done(PairOutcome::Cancelled);
    Some(outgoing.id.clone())
}

pub fn pairing_closed() {
    if let Some(id) = cancelled_if(|outgoing| matches!(outgoing.phase, Phase::Comparing { .. })) {
        crate::net::lan(LanCmd::CancelPair(id));
    }
}
