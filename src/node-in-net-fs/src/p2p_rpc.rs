use client_core::NetCmd;
use std::collections::HashMap;

fn peer_terminals() -> &'static std::sync::Mutex<HashMap<String, String>> {
    static PEER_TERMINALS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, String>>> =
        std::sync::OnceLock::new();
    PEER_TERMINALS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub fn set_peer_terminals(map: HashMap<String, String>) {
    *peer_terminals().lock().unwrap_or_else(|e| e.into_inner()) = map;
}

pub fn peer_terminal(peer_id: &str) -> Option<String> {
    peer_terminals()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(peer_id)
        .cloned()
}

fn progress_so_far() -> &'static std::sync::Mutex<HashMap<uuid::Uuid, u64>> {
    static HELD: std::sync::OnceLock<std::sync::Mutex<HashMap<uuid::Uuid, u64>>> =
        std::sync::OnceLock::new();
    HELD.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub fn fire_download_progress(transfer_id: uuid::Uuid, bytes_read: u64) {
    if let Ok(mut held) = progress_so_far().lock() {
        held.insert(transfer_id, bytes_read);
    }
}

fn forget_progress(transfer_id: uuid::Uuid) {
    if let Ok(mut held) = progress_so_far().lock() {
        held.remove(&transfer_id);
    }
}

fn progress_of(transfer_id: uuid::Uuid) -> Option<u64> {
    progress_so_far().lock().ok()?.get(&transfer_id).copied()
}

struct ProgressGuard(uuid::Uuid);

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        forget_progress(self.0);
    }
}

use crate::error::AppError;
use crate::error::RemoteFileEntry;
use nodeinnet_protocol::P2pMessage;
use tokio::sync::mpsc::Sender as TokioSender;

fn parse_date_str(date_str: &str) -> u64 {
    use chrono::TimeZone;
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S") {
        if let chrono::LocalResult::Single(dt) = chrono::Local.from_local_datetime(&naive) {
            return dt.timestamp() as u64;
        }
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M") {
        if let chrono::LocalResult::Single(dt) = chrono::Local.from_local_datetime(&naive) {
            return dt.timestamp() as u64;
        }
    }
    0
}

pub struct RemoteFileSystemRpc {
    pub net_tx: TokioSender<NetCmd>,
    pub resource_id: String,
    pub peer_id: String,
}

const PEER_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const RECEIPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const SETID_BITS: u32 = 0o6000;

pub struct PendingGuard(uuid::Uuid);

struct TempFileGuard(std::path::PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("part"));
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = crate::pending::get_pending_requests().lock() {
            pending.remove(&self.0);
        }
    }
}

impl RemoteFileSystemRpc {
    pub fn register(
        &self,
        id: uuid::Uuid,
    ) -> (
        tokio::sync::mpsc::UnboundedReceiver<P2pMessage>,
        PendingGuard,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        if let Ok(mut pending) = crate::pending::get_pending_requests().lock() {
            pending.insert(id, tx);
        }
        (rx, PendingGuard(id))
    }

    pub async fn send_to_peer_await(&self, msg: P2pMessage) -> Result<(), AppError> {
        self.net_tx
            .send(NetCmd::SendToPeer(self.peer_id.clone(), msg))
            .await
            .map_err(|_| AppError::Remote("peer link is closed".to_string()))
    }

    async fn queue(&self, msg: P2pMessage) -> Result<(), AppError> {
        tokio::time::timeout(RECEIPT_TIMEOUT, self.send_to_peer_await(msg))
            .await
            .unwrap_or_else(|_| {
                Err(AppError::Remote(format!(
                    "upload stalled: nothing left the queue for {} min",
                    RECEIPT_TIMEOUT.as_secs() / 60
                )))
            })
    }

    pub fn send_to_peer(&self, msg: P2pMessage) -> Result<(), AppError> {
        self.net_tx
            .try_send(NetCmd::SendToPeer(self.peer_id.clone(), msg))
            .map_err(|e| match e {
                tokio::sync::mpsc::error::TrySendError::Full(_) => AppError::Remote(
                    "peer command queue is full - the link is not keeping up".to_string(),
                ),
                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                    AppError::Remote("peer link is closed".to_string())
                }
            })
    }

    pub async fn await_transfer_end(
        mut rx: tokio::sync::mpsc::UnboundedReceiver<P2pMessage>,
        what: &str,
    ) -> Result<P2pMessage, AppError> {
        loop {
            match tokio::time::timeout(PEER_RESPONSE_TIMEOUT, rx.recv()).await {
                Ok(Some(msg)) => match msg {
                    P2pMessage::FileTransferComplete { .. } => return Ok(msg),
                    P2pMessage::FileTransferResponse {
                        status: nodeinnet_protocol::FileTransferStatus::Rejected { .. },
                        ..
                    } => return Ok(msg),
                    _ => continue,
                },
                Ok(None) => {
                    return Err(AppError::Remote(format!("{what}: the request was dropped")))
                }
                Err(_) => {
                    return Err(AppError::Remote(format!(
                        "{what}: no response from the peer within {}s",
                        PEER_RESPONSE_TIMEOUT.as_secs()
                    )))
                }
            }
        }
    }

    pub async fn await_peer(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<P2pMessage>,
        what: &str,
    ) -> Result<P2pMessage, AppError> {
        match tokio::time::timeout(PEER_RESPONSE_TIMEOUT, rx.recv()).await {
            Ok(Some(msg)) => Ok(msg),
            Ok(None) => Err(AppError::Remote(format!("{what}: the request was dropped"))),
            Err(_) => Err(AppError::Remote(format!(
                "{what}: no response from the peer within {}s",
                PEER_RESPONSE_TIMEOUT.as_secs()
            ))),
        }
    }

    async fn receipt(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<P2pMessage>,
    ) -> Result<(), AppError> {
        loop {
            match rx.recv().await {
                Some(P2pMessage::FileTransferReceipt { result, .. }) => {
                    return result
                        .map_err(|reason| AppError::Remote(format!("upload failed: {reason}")))
                }
                Some(_) => continue,
                None => {
                    return Err(AppError::Remote(
                        "upload: the request was dropped".to_string(),
                    ))
                }
            }
        }
    }
}

impl RemoteFileSystemRpc {
    pub fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    pub fn supports_offset_io(&self) -> bool {
        false
    }

    pub fn supports_terminal(&self) -> bool {
        peer_terminal(&self.peer_id).is_some()
    }
    pub fn content_wait(&self) -> crate::error::ContentWait {
        crate::error::net_content_wait()
    }
    pub async fn create_directory(
        &self,
        parent_path: String,
        dir_name: String,
        permissions: Option<u32>,
    ) -> Result<(), AppError> {
        let request_id = uuid::Uuid::new_v4();
        let (mut rx, _guard) = self.register(request_id);

        self.send_to_peer(P2pMessage::CreateDirectoryRequest {
            request_id,
            resource_id: self.resource_id.clone(),
            parent_path,
            dir_name,
            permissions,
        })?;

        match Self::await_peer(&mut rx, "create directory").await? {
            P2pMessage::CreateDirectoryResponse { result, .. } => result.map_err(AppError::Remote),
            _ => Err(AppError::Remote(
                "unexpected reply to create directory".to_string(),
            )),
        }
    }

    pub async fn delete_entries(&self, paths: Vec<String>) -> Result<(), AppError> {
        let worker = Self {
            net_tx: self.net_tx.clone(),
            resource_id: self.resource_id.clone(),
            peer_id: self.peer_id.clone(),
        };
        tokio::spawn(async move {
            for path in paths {
                let request_id = uuid::Uuid::new_v4();
                let (mut rx, _guard) = worker.register(request_id);

                worker
                    .send_to_peer_await(P2pMessage::DeleteEntryRequest {
                        request_id,
                        resource_id: worker.resource_id.clone(),
                        path: path.clone(),
                    })
                    .await?;

                match Self::await_peer(&mut rx, "delete entry").await? {
                    P2pMessage::DeleteEntryResponse { result, .. } => {
                        result.map_err(|e| AppError::Remote(format!("{path}: {e}")))?
                    }
                    _ => {
                        return Err(AppError::Remote(format!(
                            "{path}: unexpected reply to delete"
                        )))
                    }
                }
            }
            Ok::<(), AppError>(())
        })
        .await
        .map_err(|e| AppError::Other(format!("delete task: {e}")))?
    }

    pub async fn rename_entry(&self, path: String, new_path: String) -> Result<(), AppError> {
        let request_id = uuid::Uuid::new_v4();
        let (mut rx, _guard) = self.register(request_id);

        self.send_to_peer(P2pMessage::RenameEntryRequest {
            request_id,
            resource_id: self.resource_id.clone(),
            path,
            new_path,
        })?;

        match Self::await_peer(&mut rx, "rename entry").await? {
            P2pMessage::RenameEntryResponse { result, .. } => result.map_err(AppError::Remote),
            _ => Err(AppError::Remote("unexpected reply to rename".to_string())),
        }
    }

    pub fn request_file_download(&self, file_path: String, transfer_id: uuid::Uuid) {
        if let Err(e) = self.send_to_peer(P2pMessage::FileDownloadRequest {
            resource_id: self.resource_id.clone(),
            file_path,
            transfer_id,
            offset: 0,
            length: None,
        }) {
            crate::note(&format!("[P2P] download request not sent: {e}"));
        }
    }

    pub async fn read_file(
        &self,
        path: String,
        progress_callback: Option<Box<dyn Fn(u64) + 'static>>,
    ) -> Result<Vec<u8>, AppError> {
        let transfer_id = uuid::Uuid::new_v4();
        let _progress_guard = ProgressGuard(transfer_id);

        let (rx, _guard) = self.register(transfer_id);
        let temp_path = std::env::temp_dir().join(transfer_id.to_string());
        let _temp_guard = TempFileGuard(temp_path.clone());

        self.send_to_peer(P2pMessage::FileDownloadRequest {
            resource_id: self.resource_id.clone(),
            file_path: path,
            transfer_id,
            offset: 0,
            length: None,
        })?;

        let waiting = Self::await_transfer_end(rx, "download file");
        tokio::pin!(waiting);
        let mut told = 0u64;
        let arrived = loop {
            tokio::select! {
                done = &mut waiting => break done?,
                () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if let (Some(report), Some(far)) =
                        (progress_callback.as_ref(), progress_of(transfer_id))
                    {
                        if far > told {
                            told = far;
                            report(far);
                        }
                    }
                }
            }
        };

        match arrived {
            P2pMessage::FileTransferComplete { .. } => std::fs::read(&temp_path)
                .map_err(|e| AppError::Remote(format!("failed to read the downloaded file: {e}"))),
            P2pMessage::FileTransferResponse {
                status: nodeinnet_protocol::FileTransferStatus::Rejected { reason },
                ..
            } => Err(AppError::Remote(format!("download rejected: {reason}"))),
            other => Err(AppError::Remote(format!(
                "unexpected reply to download: {other:?}"
            ))),
        }
    }

    pub async fn write_file(
        &self,
        path: String,
        content: Vec<u8>,
        permissions: Option<u32>,
        progress_callback: Option<Box<dyn Fn(u64) + 'static>>,
    ) -> Result<(), AppError> {
        let transfer_id = uuid::Uuid::new_v4();
        let file_path = std::path::Path::new(&path);
        let file_name = file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let target_path = file_path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        let target_path = if target_path.is_empty() {
            "/".to_string()
        } else {
            target_path
        };
        let total_size = content.len() as u64;

        let (mut rx, _guard) = self.register(transfer_id);

        self.queue(P2pMessage::FileUploadRequest {
            resource_id: self.resource_id.clone(),
            target_path,
            file_name,
            total_size,
            transfer_id,
            permissions,
        })
        .await?;

        match Self::await_peer(&mut rx, "upload request").await? {
            P2pMessage::FileTransferResponse {
                status: nodeinnet_protocol::FileTransferStatus::Accepted { .. },
                ..
            } => {}
            P2pMessage::FileTransferResponse {
                status: nodeinnet_protocol::FileTransferStatus::Rejected { reason },
                ..
            } => return Err(AppError::Remote(format!("upload rejected: {reason}"))),
            P2pMessage::FileTransferReceipt {
                result: Err(reason),
                ..
            } => return Err(AppError::Remote(format!("upload failed: {reason}"))),
            _ => return Err(AppError::Remote("unexpected reply to upload".to_string())),
        }

        let worker = Self {
            net_tx: self.net_tx.clone(),
            resource_id: self.resource_id.clone(),
            peer_id: self.peer_id.clone(),
        };
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<u64>(8);
        let mut upload = tokio::spawn(async move {
            let mut offset = 0usize;
            let mut last_sent = std::time::Instant::now() - std::time::Duration::from_millis(200);
            for chunk_data in content.chunks(16 * 1024) {
                worker
                    .queue(P2pMessage::FileChunk {
                        transfer_id,
                        offset: offset as u64,
                        data: chunk_data.to_vec(),
                    })
                    .await?;
                offset += chunk_data.len();
                if last_sent.elapsed().as_millis() >= 100 {
                    let _ = progress_tx.try_send(offset as u64);
                    last_sent = std::time::Instant::now();
                }
            }
            let _ = progress_tx.try_send(offset as u64);
            worker
                .queue(P2pMessage::FileTransferComplete {
                    transfer_id,
                    checksum: None,
                })
                .await?;
            Ok::<(), AppError>(())
        });

        let streamed = loop {
            tokio::select! {
                biased;
                Some(bytes) = progress_rx.recv() => {
                    if let Some(ref cb) = progress_callback {
                        cb(bytes);
                    }
                }
                done = &mut upload => break done,
                answer = Self::receipt(&mut rx) => {
                    upload.abort();
                    return Err(answer.err().unwrap_or_else(|| {
                        AppError::Remote("unexpected reply to upload".to_string())
                    }));
                }
            }
        };
        streamed.map_err(|e| AppError::Other(format!("upload task: {e}")))??;

        tokio::time::timeout(RECEIPT_TIMEOUT, Self::receipt(&mut rx))
            .await
            .unwrap_or_else(|_| {
                Err(AppError::Remote(format!(
                    "upload: no confirmation from the peer within {} min",
                    RECEIPT_TIMEOUT.as_secs() / 60
                )))
            })
    }

    pub async fn list_dir(&self, path: String) -> Result<Vec<RemoteFileEntry>, AppError> {
        let request_id = uuid::Uuid::new_v4();
        let (mut rx, _guard) = self.register(request_id);

        self.send_to_peer(P2pMessage::RequestEntries {
            request_id,
            path,
            resource_id: self.resource_id.clone(),
        })?;

        match Self::await_peer(&mut rx, "list directory").await? {
            P2pMessage::EntriesResponse {
                directories,
                files,
                directories_with_dates,
                files_with_dates,
                directories_permissions,
                files_permissions,
                ..
            } => {
                let mut entries = Vec::new();
                if let Some(dw) = directories_with_dates {
                    for (i, d) in dw.into_iter().enumerate() {
                        let permissions = directories_permissions
                            .as_ref()
                            .and_then(|v| v.get(i).copied().flatten())
                            .map(|mode| mode & !SETID_BITS);
                        entries.push(RemoteFileEntry {
                            name: d.0,
                            is_dir: true,
                            size: 0,
                            modified: parse_date_str(&d.1),
                            permissions,
                            extra: Vec::new(),
                        });
                    }
                } else {
                    for (i, d) in directories.into_iter().enumerate() {
                        let permissions = directories_permissions
                            .as_ref()
                            .and_then(|v| v.get(i).copied().flatten())
                            .map(|mode| mode & !SETID_BITS);
                        entries.push(RemoteFileEntry {
                            name: d,
                            is_dir: true,
                            size: 0,
                            modified: 0,
                            permissions,
                            extra: Vec::new(),
                        });
                    }
                }

                if let Some(fw) = files_with_dates {
                    for (i, f) in fw.into_iter().enumerate() {
                        let permissions = files_permissions
                            .as_ref()
                            .and_then(|v| v.get(i).copied().flatten())
                            .map(|mode| mode & !SETID_BITS);
                        entries.push(RemoteFileEntry {
                            name: f.0,
                            is_dir: false,
                            size: f.1,
                            modified: parse_date_str(&f.2),
                            permissions,
                            extra: Vec::new(),
                        });
                    }
                } else {
                    for (i, f) in files.into_iter().enumerate() {
                        let permissions = files_permissions
                            .as_ref()
                            .and_then(|v| v.get(i).copied().flatten())
                            .map(|mode| mode & !SETID_BITS);
                        entries.push(RemoteFileEntry {
                            name: f.0,
                            is_dir: false,
                            size: f.1,
                            modified: 0,
                            permissions,
                            extra: Vec::new(),
                        });
                    }
                }

                Ok(entries)
            }
            _ => Err(AppError::Remote(
                "unexpected reply to directory listing".to_string(),
            )),
        }
    }

    pub async fn set_permissions(&self, path: String, permissions: u32) -> Result<(), AppError> {
        let request_id = uuid::Uuid::new_v4();
        let (mut rx, _guard) = self.register(request_id);

        self.send_to_peer(P2pMessage::SetPermissionsRequest {
            request_id,
            resource_id: self.resource_id.clone(),
            path,
            permissions,
        })?;

        match Self::await_peer(&mut rx, "set permissions").await? {
            P2pMessage::SetPermissionsResponse { result, .. } => result.map_err(AppError::Remote),
            _ => Err(AppError::Remote(
                "unexpected reply to set permissions".to_string(),
            )),
        }
    }

    pub fn connection_id(&self) -> Option<String> {
        Some(format!("p2p://{}@{}", self.peer_id, self.resource_id))
    }

    pub fn display_name(&self) -> Option<String> {
        let resource =
            nodeinnet_protocol::get_known_resource_name(&self.peer_id, &self.resource_id);
        let peer = nodeinnet_protocol::get_known_peer_name(&self.peer_id);
        match (resource, peer) {
            (Some(resource), Some(peer)) => Some(format!("{resource} ({peer})")),
            (Some(resource), None) => Some(resource),
            (None, Some(peer)) => Some(peer),
            (None, None) => None,
        }
    }

    pub fn get_icon(&self, path: &str) -> String {
        let path_norm = path.replace('\\', "/");
        if path_norm == "/" || path_norm.is_empty() {
            "/com/icecommander/gtk/connect.svg".to_string()
        } else {
            "/com/icecommander/gtk/folder.svg".to_string()
        }
    }
}
