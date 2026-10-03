//! Blocking is fine here: the host calls a connection's entries on a worker thread.

use crate::error::RemoteFileEntry;
use crate::p2p_rpc::RemoteFileSystemRpc;
use ic_plugin_api::{
    IcBytes, IcConnectionVTable, IcDirEntry, IcFsHandle, IcFsVTable, IcListing, IC_ERR_INIT_FAILED,
    IC_OK,
};
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

struct Mounted {
    rpc: RemoteFileSystemRpc,
    trouble: RefCell<String>,
    // Kept alive while the host reads what `list`, `read` and `last_error` answered.
    names: RefCell<Vec<CString>>,
    rows: RefCell<Vec<IcDirEntry>>,
    bytes: RefCell<Vec<u8>>,
    said: RefCell<CString>,
}

fn held(handle: IcFsHandle) -> Option<&'static Mounted> {
    if handle.is_null() {
        return None;
    }
    Some(unsafe { &*(handle as *const Mounted) })
}

fn path_of(path: *const c_char) -> String {
    if path.is_null() {
        return "/".to_string();
    }
    unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .to_string()
}

fn awaited<T>(
    work: impl std::future::Future<Output = Result<T, crate::error::AppError>>,
) -> Result<T, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|why| format!("no runtime: {why}"))?;
    runtime.block_on(work).map_err(|why| why.to_string())
}

fn blamed(mounted: &Mounted, why: String) {
    crate::note(&format!("peer filesystem: {why}"));
    *mounted.trouble.borrow_mut() = why;
}

extern "C" fn open(settings: *const u8, len: u64, _user_data: *mut c_void) -> IcFsHandle {
    if settings.is_null() || len == 0 {
        return std::ptr::null_mut();
    }
    let raw = unsafe { std::slice::from_raw_parts(settings, len as usize) };
    let Ok(held) = serde_json::from_slice::<serde_json::Value>(raw) else {
        return std::ptr::null_mut();
    };
    let peer_id = held["peer_id"].as_str().unwrap_or_default().to_string();
    let resource_id = held["resource_id"].as_str().unwrap_or_default().to_string();
    let Some(net_tx) = crate::net::sender() else {
        crate::note("a peer share was opened while no account is signed in");
        return std::ptr::null_mut();
    };
    if peer_id.is_empty() || resource_id.is_empty() {
        return std::ptr::null_mut();
    }
    // Dial first so the link is up before the first listing is asked for.
    let _ = net_tx.try_send(client_core::NetCmd::Call(peer_id.clone()));
    crate::note(&format!("mounting {resource_id} on {peer_id}"));
    let mounted = Box::new(Mounted {
        rpc: RemoteFileSystemRpc {
            net_tx,
            resource_id,
            peer_id,
        },
        trouble: RefCell::new(String::new()),
        names: RefCell::new(Vec::new()),
        rows: RefCell::new(Vec::new()),
        bytes: RefCell::new(Vec::new()),
        said: RefCell::new(CString::default()),
    });
    Box::into_raw(mounted) as IcFsHandle
}

extern "C" fn close(handle: IcFsHandle) {
    if handle.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(handle as *mut Mounted) });
}

fn listing_of(mounted: &Mounted, entries: Vec<RemoteFileEntry>) -> IcListing {
    let names: Vec<CString> = entries
        .iter()
        .map(|entry| CString::new(entry.name.clone()).unwrap_or_default())
        .collect();
    let rows: Vec<IcDirEntry> = entries
        .iter()
        .zip(names.iter())
        .map(|(entry, name)| IcDirEntry {
            name: name.as_ptr(),
            is_dir: i32::from(entry.is_dir),
            size: entry.size,
            modified: entry.modified,
            permissions: entry.permissions.unwrap_or(0),
            has_permissions: i32::from(entry.permissions.is_some()),
        })
        .collect();
    *mounted.names.borrow_mut() = names;
    *mounted.rows.borrow_mut() = rows;
    let borrowed = mounted.rows.borrow();
    IcListing {
        items: borrowed.as_ptr(),
        count: borrowed.len() as u32,
    }
}

extern "C" fn list(handle: IcFsHandle, path: *const c_char) -> IcListing {
    let Some(mounted) = held(handle) else {
        return IcListing::EMPTY;
    };
    match awaited(mounted.rpc.list_dir(path_of(path))) {
        Ok(entries) => listing_of(mounted, entries),
        Err(why) => {
            blamed(mounted, why);
            IcListing::EMPTY
        }
    }
}

extern "C" fn read(handle: IcFsHandle, path: *const c_char) -> IcBytes {
    let Some(mounted) = held(handle) else {
        return IcBytes::EMPTY;
    };
    match awaited(mounted.rpc.read_file(path_of(path), None)) {
        Ok(bytes) => {
            *mounted.bytes.borrow_mut() = bytes;
            let borrowed = mounted.bytes.borrow();
            IcBytes {
                data: borrowed.as_ptr(),
                len: borrowed.len() as u64,
            }
        }
        Err(why) => {
            blamed(mounted, why);
            IcBytes::EMPTY
        }
    }
}

extern "C" fn is_read_only(_handle: IcFsHandle) -> c_int {
    0
}

extern "C" fn last_error(handle: IcFsHandle) -> *const c_char {
    let Some(mounted) = held(handle) else {
        return std::ptr::null();
    };
    let said = CString::new(mounted.trouble.borrow().clone()).unwrap_or_default();
    *mounted.said.borrow_mut() = said;
    mounted.said.borrow().as_ptr()
}

extern "C" fn write(handle: IcFsHandle, path: *const c_char, bytes: *const u8, len: u64) -> c_int {
    let Some(mounted) = held(handle) else {
        return IC_ERR_INIT_FAILED;
    };
    let body = if bytes.is_null() || len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes, len as usize) }.to_vec()
    };
    match awaited(mounted.rpc.write_file(path_of(path), body, None, None)) {
        Ok(()) => IC_OK,
        Err(why) => {
            blamed(mounted, why);
            IC_ERR_INIT_FAILED
        }
    }
}

extern "C" fn create_dir(handle: IcFsHandle, path: *const c_char) -> c_int {
    let Some(mounted) = held(handle) else {
        return IC_ERR_INIT_FAILED;
    };
    let full = path_of(path);
    let (parent, name) = match full.rsplit_once('/') {
        Some((parent, name)) if !name.is_empty() => (parent.to_string(), name.to_string()),
        _ => ("/".to_string(), full.clone()),
    };
    let parent = if parent.is_empty() {
        "/".to_string()
    } else {
        parent
    };
    match awaited(mounted.rpc.create_directory(parent, name, None)) {
        Ok(()) => IC_OK,
        Err(why) => {
            blamed(mounted, why);
            IC_ERR_INIT_FAILED
        }
    }
}

extern "C" fn remove(handle: IcFsHandle, path: *const c_char) -> c_int {
    let Some(mounted) = held(handle) else {
        return IC_ERR_INIT_FAILED;
    };
    match awaited(mounted.rpc.delete_entries(vec![path_of(path)])) {
        Ok(()) => IC_OK,
        Err(why) => {
            blamed(mounted, why);
            IC_ERR_INIT_FAILED
        }
    }
}

extern "C" fn rename(handle: IcFsHandle, from: *const c_char, to: *const c_char) -> c_int {
    let Some(mounted) = held(handle) else {
        return IC_ERR_INIT_FAILED;
    };
    match awaited(mounted.rpc.rename_entry(path_of(from), path_of(to))) {
        Ok(()) => IC_OK,
        Err(why) => {
            blamed(mounted, why);
            IC_ERR_INIT_FAILED
        }
    }
}

struct Shell {
    resource_id: String,
    input_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    resize_tx: tokio::sync::mpsc::Sender<(u16, u16)>,
    output_rx: RefCell<tokio::sync::mpsc::Receiver<Vec<u8>>>,
    waiting: RefCell<Vec<u8>>,
    ended: std::cell::Cell<bool>,
}

extern "C" fn shell_available(handle: IcFsHandle) -> c_int {
    let Some(mounted) = held(handle) else {
        return 0;
    };
    i32::from(crate::p2p_rpc::peer_terminal(&mounted.rpc.peer_id).is_some())
}

extern "C" fn shell_open(
    handle: IcFsHandle,
    _cwd: *const c_char,
    rows: u32,
    cols: u32,
) -> *mut c_void {
    let Some(mounted) = held(handle) else {
        return std::ptr::null_mut();
    };
    let Some(resource_id) = crate::p2p_rpc::peer_terminal(&mounted.rpc.peer_id) else {
        crate::note("this peer shares no terminal");
        return std::ptr::null_mut();
    };
    crate::note(&format!("opening a shell on {resource_id}"));
    let session = crate::p2p_shell::open_p2p_shell(
        mounted.rpc.net_tx.clone(),
        mounted.rpc.peer_id.clone(),
        resource_id.clone(),
        rows as u16,
        cols as u16,
    );
    Box::into_raw(Box::new(Shell {
        resource_id,
        input_tx: session.input_tx,
        resize_tx: session.resize_tx,
        output_rx: RefCell::new(session.output_rx),
        waiting: RefCell::new(Vec::new()),
        ended: std::cell::Cell::new(false),
    })) as *mut c_void
}

fn shell_of(shell: *mut c_void) -> Option<&'static Shell> {
    if shell.is_null() {
        return None;
    }
    Some(unsafe { &*(shell as *const Shell) })
}

/// Never blocks; a null slice is how the host learns the session ended.
extern "C" fn shell_read(shell: *mut c_void) -> IcBytes {
    let Some(held) = shell_of(shell) else {
        return IcBytes {
            data: std::ptr::null(),
            len: 0,
        };
    };
    if held.ended.get() {
        return IcBytes {
            data: std::ptr::null(),
            len: 0,
        };
    }
    let mut waiting = held.waiting.borrow_mut();
    let mut incoming = held.output_rx.borrow_mut();
    waiting.clear();
    loop {
        match incoming.try_recv() {
            Ok(more) => waiting.extend_from_slice(&more),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                held.ended.set(true);
                break;
            }
        }
    }
    if held.ended.get() && waiting.is_empty() {
        return IcBytes {
            data: std::ptr::null(),
            len: 0,
        };
    }
    IcBytes {
        data: waiting.as_ptr(),
        len: waiting.len() as u64,
    }
}

extern "C" fn shell_write(shell: *mut c_void, bytes: *const u8, len: u64) -> c_int {
    let Some(held) = shell_of(shell) else {
        return IC_ERR_INIT_FAILED;
    };
    if bytes.is_null() || len == 0 {
        return IC_OK;
    }
    let typed = unsafe { std::slice::from_raw_parts(bytes, len as usize) }.to_vec();
    match held.input_tx.try_send(typed) {
        Ok(()) => IC_OK,
        Err(_) => IC_ERR_INIT_FAILED,
    }
}

extern "C" fn shell_resize(shell: *mut c_void, rows: u32, cols: u32) -> c_int {
    let Some(held) = shell_of(shell) else {
        return IC_ERR_INIT_FAILED;
    };
    match held.resize_tx.try_send((rows as u16, cols as u16)) {
        Ok(()) => IC_OK,
        Err(_) => IC_ERR_INIT_FAILED,
    }
}

extern "C" fn shell_close(shell: *mut c_void) {
    if shell.is_null() {
        return;
    }
    let held = unsafe { Box::from_raw(shell as *mut Shell) };
    crate::note(&format!("closing the shell on {}", held.resource_id));
    // Dropping input_tx is what makes p2p_shell send StopTerminal to the peer.
    drop(held);
}

extern "C" fn never_opened_inside_a_file(
    _source: ic_plugin_api::IcFsSource,
    _path: *const c_char,
    _user_data: *mut c_void,
) -> IcFsHandle {
    std::ptr::null_mut()
}

pub fn filesystem() -> IcFsVTable {
    IcFsVTable {
        struct_size: std::mem::size_of::<IcFsVTable>() as u32,
        open_in: never_opened_inside_a_file,
        close,
        list,
        read,
        is_read_only,
        last_error,
        write: Some(write),
        create_dir: Some(create_dir),
        remove: Some(remove),
        rename: Some(rename),
        shell_open: Some(shell_open),
        shell_read: Some(shell_read),
        shell_write: Some(shell_write),
        shell_resize: Some(shell_resize),
        shell_close: Some(shell_close),
        shell_available: Some(shell_available),
        columns: None,
        list_rows: None,
        action_state: None,
        cell_clicked: None,
        set_permissions: None,
    }
}

pub fn connection(fs: *const IcFsVTable) -> IcConnectionVTable {
    IcConnectionVTable {
        struct_size: std::mem::size_of::<IcConnectionVTable>() as u32,
        open,
        fs,
        describe: None,
        on_event: None,
    }
}
