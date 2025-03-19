use anyhow::Result;
use serde::{Serialize, Deserialize};
use std::collections::{HashMap, HashSet, BinaryHeap, VecDeque};
use tokio::sync::{mpsc, Semaphore};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::cmp::Ordering;
use crate::crypto::CryptoService;

const MESSAGE_EXPIRY_SECONDS: u64 = 60;
const MAX_MESSAGE_SIZE: usize = 1024 * 1024; // 1MB
const DELIVERY_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_GROUP_SIZE: usize = 50;
const CHUNK_SIZE: usize = 1024 * 64; // 64KB chunks
const MAX_CHUNK_RETRIES: u32 = 3;
const MAX_PARALLEL_CHUNKS: usize = 4;
const RATE_LIMIT_CHUNKS_PER_SEC: u32 = 10;
const RATE_LIMIT_WINDOW_MS: u64 = 1000;
const MIN_BANDWIDTH_BYTES_PER_SEC: u64 = 1024; // 1KB/s
const MAX_BANDWIDTH_BYTES_PER_SEC: u64 = 1024 * 1024; // 1MB/s
const PROGRESS_UPDATE_INTERVAL_MS: u64 = 1000;
const PRIORITY_LEVELS: u8 = 5;
const SPEED_ADAPTATION_INTERVAL_MS: u64 = 5000;
const MIN_SPEED_THRESHOLD: f64 = 0.1; // 10% of target speed
const MAX_SPEED_THRESHOLD: f64 = 1.5; // 150% of target speed
const QUEUE_CHECK_INTERVAL_MS: u64 = 1000;
const ERROR_RECOVERY_DELAY_MS: u64 = 1000;
const MAX_ERROR_RETRIES: u32 = 3;

#[derive(Debug, Serialize, Deserialize)]
pub enum ProtocolMessage {
    Text {
        content: String,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
        delivery_id: Option<String>,
    },
    Group {
        group_id: String,
        content: String,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
        delivery_id: Option<String>,
        sender_ephemeral_id: String,
    },
    File {
        name: String,
        size: u64,
        hash: String,
        chunks: Vec<FileChunk>,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
        delivery_id: Option<String>,
    },
    FileProgress {
        file_id: String,
        total_chunks: u32,
        received_chunks: HashSet<u32>,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileResume {
        file_id: String,
        missing_chunks: HashSet<u32>,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    Control {
        command: ControlCommand,
        data: Vec<u8>,
        ephemeral_id: String,
    },
    FileChunkRequest {
        file_id: String,
        chunk_indices: Vec<u32>,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileChunkResponse {
        file_id: String,
        chunks: Vec<FileChunk>,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    RateLimitExceeded {
        file_id: String,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileTransferCancel {
        file_id: String,
        reason: String,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileTransferPriority {
        file_id: String,
        priority: u8,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileTransferBandwidth {
        file_id: String,
        bandwidth_limit: u64,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileTransferProgress {
        file_id: String,
        bytes_transferred: u64,
        total_bytes: u64,
        speed: f64,
        estimated_time: u64,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileTransferResume {
        file_id: String,
        resume_point: u64,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileTransferSpeedAdapt {
        file_id: String,
        target_speed: f64,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileTransferQueueUpdate {
        file_id: String,
        queue_position: u32,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
    FileTransferError {
        file_id: String,
        error_type: String,
        retry_count: u32,
        timestamp: u64,
        expiry: u64,
        ephemeral_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileChunk {
    pub index: u32,
    pub data: Vec<u8>,
    pub hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ControlCommand {
    CircuitExtend,
    CircuitExtended,
    CircuitDestroy,
    KeepAlive,
    Error,
    MessageReceived(String),
    MessageExpired(String),
    DeliveryConfirmed(String),
    DeliveryFailed(String, String),
    GroupJoin(String),
    GroupLeave(String),
    GroupMessageReceived(String, String),
    GroupMessageExpired(String, String),
    ChunkReceived(String, u32), // file_id, chunk_index
    ChunkMissing(String, u32), // file_id, chunk_index
    FileComplete(String), // file_id
    FileFailed(String, String), // file_id, reason
}

#[derive(Debug)]
pub struct ProtocolHandler {
    crypto_service: CryptoService,
    message_sender: mpsc::Sender<ProtocolMessage>,
    message_receiver: mpsc::Receiver<ProtocolMessage>,
    active_streams: HashMap<String, StreamState>,
    message_tracker: HashMap<String, MessageInfo>,
    delivery_tracker: HashMap<String, DeliveryInfo>,
    group_members: HashMap<String, GroupInfo>,
    file_transfers: HashMap<String, FileTransferInfo>,
    chunk_semaphore: Semaphore,
    rate_limiter: RateLimiter,
    bandwidth_manager: BandwidthManager,
    transfer_priorities: BinaryHeap<FileTransferPriority>,
    transfer_queue: TransferQueue,
}

#[derive(Debug)]
struct StreamState {
    stream_id: String,
    peer_id: String,
    protocol: String,
    is_active: bool,
}

#[derive(Debug)]
struct MessageInfo {
    timestamp: u64,
    expiry: u64,
    is_viewed: bool,
    delivery_id: Option<String>,
}

#[derive(Debug)]
struct DeliveryInfo {
    message_id: String,
    timestamp: u64,
    is_confirmed: bool,
    retry_count: u32,
}

#[derive(Debug)]
struct GroupInfo {
    members: HashSet<String>,
    created_at: u64,
    last_activity: u64,
    ephemeral_id: String,
}

#[derive(Debug, Eq, PartialEq)]
struct FileTransferPriority {
    priority: u8,
    start_time: u64,
    file_id: String,
}

impl Ord for FileTransferPriority {
    fn cmp(&self, other: &Self) -> Ordering {
        other.priority.cmp(&self.priority)
            .then_with(|| self.start_time.cmp(&other.start_time))
    }
}

impl PartialOrd for FileTransferPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
struct FileTransferInfo {
    file_id: String,
    total_chunks: u32,
    received_chunks: HashSet<u32>,
    start_time: u64,
    last_update: u64,
    is_complete: bool,
    retry_count: u32,
    pending_chunks: HashSet<u32>,
    active_chunks: HashSet<u32>,
    priority: u8,
    bandwidth_limit: u64,
    bytes_transferred: u64,
    last_progress_update: u64,
    speed: f64,
    is_cancelled: bool,
    resume_point: u64,
    target_speed: f64,
    error_count: u32,
    last_error: Option<String>,
    last_speed_adaptation: u64,
}

#[derive(Debug)]
struct BandwidthManager {
    active_transfers: HashMap<String, u64>,
    total_bandwidth: u64,
    last_update: u64,
}

impl BandwidthManager {
    fn new(total_bandwidth: u64) -> Self {
        Self {
            active_transfers: HashMap::new(),
            total_bandwidth,
            last_update: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        }
    }

    fn allocate_bandwidth(&mut self, file_id: &str, requested: u64) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        // Clean up old allocations
        self.active_transfers.retain(|_, &mut last_update| {
            now - last_update < RATE_LIMIT_WINDOW_MS
        });

        // Calculate available bandwidth
        let used_bandwidth: u64 = self.active_transfers.values().sum();
        let available = self.total_bandwidth.saturating_sub(used_bandwidth);

        // Allocate bandwidth
        let allocated = requested.min(available);
        if allocated > 0 {
            self.active_transfers.insert(file_id.to_string(), now);
        }

        allocated
    }

    fn release_bandwidth(&mut self, file_id: &str) {
        self.active_transfers.remove(file_id);
    }
}

impl RateLimiter {
    fn new(window_size: u64, max_chunks: u32) -> Self {
        Self {
            chunks_sent: HashMap::new(),
            window_size,
            max_chunks,
        }
    }

    fn can_send_chunks(&mut self, file_id: &str) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let chunks = self.chunks_sent.entry(file_id.to_string()).or_insert_with(Vec::new);
        
        // Remove old timestamps
        chunks.retain(|&t| now - t < self.window_size);
        
        // Check if we can send more chunks
        if chunks.len() as u32 >= self.max_chunks {
            return false;
        }
        
        chunks.push(now);
        true
    }

    fn cleanup(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        for chunks in self.chunks_sent.values_mut() {
            chunks.retain(|&t| now - t < self.window_size);
        }
        
        // Remove empty entries
        self.chunks_sent.retain(|_, chunks| !chunks.is_empty());
    }
}

#[derive(Debug)]
struct TransferQueue {
    queue: VecDeque<String>,
    last_check: u64,
}

impl TransferQueue {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            last_check: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        }
    }

    fn add_transfer(&mut self, file_id: String) {
        if !self.queue.contains(&file_id) {
            self.queue.push_back(file_id);
        }
    }

    fn remove_transfer(&mut self, file_id: &str) {
        self.queue.retain(|id| id != file_id);
    }

    fn get_position(&self, file_id: &str) -> Option<u32> {
        self.queue.iter().position(|id| id == file_id).map(|p| p as u32)
    }

    fn next_transfer(&mut self) -> Option<String> {
        self.queue.pop_front()
    }
}

impl ProtocolHandler {
    pub fn new(crypto_service: CryptoService) -> Self {
        let (message_sender, message_receiver) = mpsc::channel(100);
        Self {
            crypto_service,
            message_sender,
            message_receiver,
            active_streams: HashMap::new(),
            message_tracker: HashMap::new(),
            delivery_tracker: HashMap::new(),
            group_members: HashMap::new(),
            file_transfers: HashMap::new(),
            chunk_semaphore: Semaphore::new(MAX_PARALLEL_CHUNKS),
            rate_limiter: RateLimiter::new(RATE_LIMIT_WINDOW_MS, RATE_LIMIT_CHUNKS_PER_SEC),
            bandwidth_manager: BandwidthManager::new(MAX_BANDWIDTH_BYTES_PER_SEC),
            transfer_priorities: BinaryHeap::new(),
            transfer_queue: TransferQueue::new(),
        }
    }

    pub async fn handle_message(&mut self, peer_id: String, message: Vec<u8>) -> Result<()> {
        let decrypted = self.crypto_service.decrypt_message(&message)?;
        let protocol_message: ProtocolMessage = serde_json::from_slice(&decrypted)?;

        if decrypted.len() > MAX_MESSAGE_SIZE {
            return Err(anyhow::anyhow!("Message too large"));
        }

        match &protocol_message {
            ProtocolMessage::Text { content, timestamp, expiry, ephemeral_id, delivery_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    self.handle_expired_message(ephemeral_id.clone()).await?;
                    return Ok(());
                }
                
                log::info!("Received text message from {}: {}", peer_id, content);
                self.track_message(ephemeral_id.clone(), *timestamp, *expiry, delivery_id.clone());
                
                if let Some(delivery_id) = delivery_id {
                    self.send_delivery_confirmation(peer_id.clone(), delivery_id.clone()).await?;
                }
                
                self.message_sender.send(protocol_message).await?;
            }
            ProtocolMessage::Group { group_id, content, timestamp, expiry, ephemeral_id, delivery_id, sender_ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    self.handle_expired_group_message(group_id.clone(), ephemeral_id.clone()).await?;
                    return Ok(());
                }

                if let Some(group) = self.group_members.get(group_id) {
                    if !group.members.contains(sender_ephemeral_id) {
                        return Err(anyhow::anyhow!("Sender is not a group member"));
                    }
                } else {
                    return Err(anyhow::anyhow!("Group does not exist"));
                }

                log::info!("Received group message from {} in group {}: {}", peer_id, group_id, content);
                self.track_message(ephemeral_id.clone(), *timestamp, *expiry, delivery_id.clone());
                
                if let Some(delivery_id) = delivery_id {
                    self.send_delivery_confirmation(peer_id.clone(), delivery_id.clone()).await?;
                }
                
                self.message_sender.send(protocol_message).await?;
            }
            ProtocolMessage::File { name, size, hash, chunks, timestamp, expiry, ephemeral_id, delivery_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    self.handle_expired_message(ephemeral_id.clone()).await?;
                    return Ok(());
                }

                if *size > MAX_MESSAGE_SIZE as u64 {
                    return Err(anyhow::anyhow!("File too large"));
                }

                // Initialize file transfer tracking
                let total_chunks = (size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64;
                self.file_transfers.insert(ephemeral_id.clone(), FileTransferInfo {
                    file_id: ephemeral_id.clone(),
                    total_chunks: total_chunks as u32,
                    received_chunks: HashSet::new(),
                    start_time: *timestamp,
                    last_update: *timestamp,
                    is_complete: false,
                    retry_count: 0,
                    pending_chunks: HashSet::from_iter(0..total_chunks as u32),
                    active_chunks: HashSet::new(),
                    priority: 0,
                    bandwidth_limit: 0,
                    bytes_transferred: 0,
                    last_progress_update: *timestamp,
                    speed: 0.0,
                    is_cancelled: false,
                    resume_point: 0,
                    target_speed: 0.0,
                    error_count: 0,
                    last_error: None,
                    last_speed_adaptation: *timestamp,
                });

                // Verify chunks
                for chunk in chunks {
                    let chunk_hash = self.crypto_service.hash_data(&chunk.data);
                    if chunk_hash != chunk.hash {
                        return Err(anyhow::anyhow!("Invalid chunk hash"));
                    }
                }

                log::info!("Received file message from {}: {} ({} bytes)", peer_id, name, size);
                self.track_message(ephemeral_id.clone(), *timestamp, *expiry, delivery_id.clone());
                
                if let Some(delivery_id) = delivery_id {
                    self.send_delivery_confirmation(peer_id.clone(), delivery_id.clone()).await?;
                }
                
                self.message_sender.send(protocol_message).await?;
            }
            ProtocolMessage::FileProgress { file_id, total_chunks, received_chunks, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    transfer.received_chunks.extend(received_chunks);
                    transfer.last_update = *timestamp;
                    transfer.is_complete = transfer.received_chunks.len() as u32 == *total_chunks;

                    // Send progress update
                    let progress = ProtocolMessage::FileProgress {
                        file_id: file_id.clone(),
                        total_chunks: *total_chunks,
                        received_chunks: transfer.received_chunks.clone(),
                        timestamp: *timestamp,
                        expiry: *expiry,
                        ephemeral_id: self.generate_ephemeral_id(),
                    };
                    self.message_sender.send(progress).await?;
                }
            }
            ProtocolMessage::FileResume { file_id, missing_chunks, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    transfer.resume_point = *timestamp;
                    transfer.error_count = 0;
                    transfer.last_error = None;
                    self.transfer_queue.add_transfer(file_id.clone());
                    
                    // Request chunks from resume point
                    let chunks_to_request: Vec<u32> = (transfer.resume_point..transfer.total_chunks)
                        .filter(|&i| !transfer.received_chunks.contains(&i))
                        .collect();
                    
                    if !chunks_to_request.is_empty() {
                        let request = ProtocolMessage::FileChunkRequest {
                            file_id: file_id.clone(),
                            chunk_indices: chunks_to_request,
                            timestamp: *timestamp,
                            expiry: *expiry,
                            ephemeral_id: self.generate_ephemeral_id(),
                        };
                        self.send_message(peer_id.clone(), request).await?;
                    }
                }
            }
            ProtocolMessage::Control { command, data, ephemeral_id } => {
                self.handle_control_message(peer_id, command.clone(), data.clone(), ephemeral_id.clone()).await?;
            }
            ProtocolMessage::FileChunkRequest { file_id, chunk_indices, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                // Check rate limit
                if !self.rate_limiter.can_send_chunks(file_id) {
                    let rate_limit_msg = ProtocolMessage::RateLimitExceeded {
                        file_id: file_id.clone(),
                        timestamp: *timestamp,
                        expiry: *expiry,
                        ephemeral_id: self.generate_ephemeral_id(),
                    };
                    self.message_sender.send(rate_limit_msg).await?;
                    return Ok(());
                }

                // Process chunk request
                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    for &chunk_index in chunk_indices {
                        if !transfer.received_chunks.contains(&chunk_index) {
                            transfer.pending_chunks.insert(chunk_index);
                        }
                    }
                    self.process_pending_chunks(file_id.clone(), peer_id.clone()).await?;
                }
            }
            ProtocolMessage::FileChunkResponse { file_id, chunks, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    for chunk in chunks {
                        transfer.received_chunks.insert(chunk.index);
                        transfer.active_chunks.remove(&chunk.index);
                        transfer.pending_chunks.remove(&chunk.index);
                    }
                    transfer.last_update = *timestamp;

                    // Check if file is complete
                    if transfer.received_chunks.len() as u32 == transfer.total_chunks {
                        transfer.is_complete = true;
                        let notification = ProtocolMessage::Control {
                            command: ControlCommand::FileComplete(file_id.clone()),
                            data: vec![],
                            ephemeral_id: self.generate_ephemeral_id(),
                        };
                        self.message_sender.send(notification).await?;
                    }
                }
            }
            ProtocolMessage::RateLimitExceeded { file_id, timestamp, expiry, ephemeral_id } => {
                // Handle rate limit exceeded
                log::warn!("Rate limit exceeded for file: {}", file_id);
                Ok(())
            }
            ProtocolMessage::FileTransferCancel { file_id, reason, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    transfer.is_cancelled = true;
                    self.bandwidth_manager.release_bandwidth(file_id);
                    self.transfer_priorities.retain(|p| p.file_id != *file_id);
                    
                    let notification = ProtocolMessage::Control {
                        command: ControlCommand::FileFailed(file_id.clone(), reason.clone()),
                        data: vec![],
                        ephemeral_id: self.generate_ephemeral_id(),
                    };
                    self.message_sender.send(notification).await?;
                }
            }
            ProtocolMessage::FileTransferPriority { file_id, priority, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    transfer.priority = *priority;
                    self.transfer_priorities.push(FileTransferPriority {
                        priority: *priority,
                        start_time: transfer.start_time,
                        file_id: file_id.clone(),
                    });
                }
            }
            ProtocolMessage::FileTransferBandwidth { file_id, bandwidth_limit, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    transfer.bandwidth_limit = *bandwidth_limit;
                    let allocated = self.bandwidth_manager.allocate_bandwidth(file_id, *bandwidth_limit);
                    if allocated != *bandwidth_limit {
                        log::warn!("Could not allocate requested bandwidth for file: {}", file_id);
                    }
                }
            }
            ProtocolMessage::FileTransferProgress { file_id, bytes_transferred, total_bytes, speed, estimated_time, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    transfer.bytes_transferred = *bytes_transferred;
                    transfer.speed = *speed;
                    transfer.last_progress_update = *timestamp;
                }
            }
            ProtocolMessage::FileTransferSpeedAdapt { file_id, target_speed, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    transfer.target_speed = *target_speed;
                    transfer.last_speed_adaptation = *timestamp;
                    
                    // Adjust bandwidth based on target speed
                    let bandwidth_limit = (*target_speed * CHUNK_SIZE as f64) as u64;
                    transfer.bandwidth_limit = bandwidth_limit;
                    self.bandwidth_manager.allocate_bandwidth(file_id, bandwidth_limit);
                }
            }
            ProtocolMessage::FileTransferQueueUpdate { file_id, queue_position, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                // Update queue position notification
                let notification = ProtocolMessage::FileTransferQueueUpdate {
                    file_id: file_id.clone(),
                    queue_position: *queue_position,
                    timestamp: *timestamp,
                    expiry: *expiry,
                    ephemeral_id: self.generate_ephemeral_id(),
                };
                self.message_sender.send(notification).await?;
            }
            ProtocolMessage::FileTransferError { file_id, error_type, retry_count, timestamp, expiry, ephemeral_id } => {
                if self.is_message_expired(*timestamp, *expiry) {
                    return Ok(());
                }

                if let Some(transfer) = self.file_transfers.get_mut(file_id) {
                    transfer.error_count = *retry_count;
                    transfer.last_error = Some(error_type.clone());
                    
                    if *retry_count >= MAX_ERROR_RETRIES {
                        self.handle_file_failure(file_id.clone(), format!("Max error retries exceeded: {}", error_type)).await?;
                    } else {
                        // Schedule retry after delay
                        let retry_msg = ProtocolMessage::FileTransferResume {
                            file_id: file_id.clone(),
                            resume_point: transfer.bytes_transferred,
                            timestamp: *timestamp + ERROR_RECOVERY_DELAY_MS / 1000,
                            expiry: *expiry,
                            ephemeral_id: self.generate_ephemeral_id(),
                        };
                        self.message_sender.send(retry_msg).await?;
                    }
                }
            }
        }

        Ok(())
    }

    fn is_message_expired(&self, timestamp: u64, expiry: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now > timestamp + expiry
    }

    async fn handle_expired_message(&mut self, ephemeral_id: String) -> Result<()> {
        log::warn!("Received expired message: {}", ephemeral_id);
        self.message_tracker.remove(&ephemeral_id);
        
        let response = ProtocolMessage::Control {
            command: ControlCommand::MessageExpired(ephemeral_id),
            data: vec![],
            ephemeral_id: self.generate_ephemeral_id(),
        };
        
        self.send_message("sender".to_string(), response).await?;
        Ok(())
    }

    fn track_message(&mut self, ephemeral_id: String, timestamp: u64, expiry: u64, delivery_id: Option<String>) {
        self.message_tracker.insert(ephemeral_id.clone(), MessageInfo {
            timestamp,
            expiry,
            is_viewed: false,
            delivery_id: delivery_id.clone(),
        });

        if let Some(delivery_id) = delivery_id {
            self.delivery_tracker.insert(delivery_id, DeliveryInfo {
                message_id: ephemeral_id,
                timestamp,
                is_confirmed: false,
                retry_count: 0,
            });
        }
    }

    async fn handle_control_message(&mut self, peer_id: String, command: ControlCommand, data: Vec<u8>, ephemeral_id: String) -> Result<()> {
        match command {
            ControlCommand::GroupJoin(group_id) => {
                self.handle_group_join(peer_id, group_id).await?;
            }
            ControlCommand::GroupLeave(group_id) => {
                self.handle_group_leave(peer_id, group_id).await?;
            }
            ControlCommand::GroupMessageReceived(group_id, message_id) => {
                self.handle_group_message_received(group_id, message_id).await?;
            }
            ControlCommand::GroupMessageExpired(group_id, message_id) => {
                self.handle_group_message_expired(group_id, message_id).await?;
            }
            ControlCommand::CircuitExtend => {
                self.handle_circuit_extension(peer_id, data).await?;
            }
            ControlCommand::CircuitExtended => {
                self.handle_circuit_extension_confirmation(peer_id, data).await?;
            }
            ControlCommand::CircuitDestroy => {
                self.handle_circuit_destruction(peer_id, data).await?;
            }
            ControlCommand::KeepAlive => {
                self.handle_keep_alive(peer_id).await?;
            }
            ControlCommand::Error => {
                self.handle_error(peer_id, data).await?;
            }
            ControlCommand::MessageReceived(recv_id) => {
                self.handle_message_received(recv_id).await?;
            }
            ControlCommand::MessageExpired(exp_id) => {
                self.handle_message_expired(exp_id).await?;
            }
            ControlCommand::DeliveryConfirmed(delivery_id) => {
                self.handle_delivery_confirmation(delivery_id).await?;
            }
            ControlCommand::DeliveryFailed(delivery_id, reason) => {
                self.handle_delivery_failure(delivery_id, reason).await?;
            }
            ControlCommand::ChunkReceived(file_id, chunk_index) => {
                self.handle_chunk_received(file_id, chunk_index).await?;
            }
            ControlCommand::ChunkMissing(file_id, chunk_index) => {
                self.handle_chunk_missing(file_id, chunk_index).await?;
            }
            ControlCommand::FileComplete(file_id) => {
                self.handle_file_completion(file_id).await?;
            }
            ControlCommand::FileFailed(file_id, reason) => {
                self.handle_file_failure(file_id, reason).await?;
            }
        }
        Ok(())
    }

    async fn handle_message_received(&mut self, ephemeral_id: String) -> Result<()> {
        if let Some(info) = self.message_tracker.get_mut(&ephemeral_id) {
            info.is_viewed = true;
            tokio::spawn({
                let ephemeral_id = ephemeral_id.clone();
                let mut handler = self.clone();
                async move {
                    tokio::time::sleep(Duration::from_secs(info.expiry)).await;
                    handler.message_tracker.remove(&ephemeral_id);
                }
            });
        }
        Ok(())
    }

    async fn handle_message_expired(&mut self, ephemeral_id: String) -> Result<()> {
        self.message_tracker.remove(&ephemeral_id);
        Ok(())
    }

    fn generate_ephemeral_id(&self) -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let id: u64 = rng.gen();
        format!("{:x}", id)
    }

    pub async fn send_message(&mut self, peer_id: String, message: ProtocolMessage) -> Result<()> {
        let message = match message {
            ProtocolMessage::Text { content, timestamp, expiry, ephemeral_id, delivery_id } => {
                ProtocolMessage::Text {
                    content,
                    timestamp,
                    expiry: expiry.unwrap_or(MESSAGE_EXPIRY_SECONDS),
                    ephemeral_id: ephemeral_id.unwrap_or_else(|| self.generate_ephemeral_id()),
                    delivery_id: delivery_id.clone(),
                }
            }
            ProtocolMessage::File { name, size, hash, chunks, timestamp, expiry, ephemeral_id, delivery_id } => {
                ProtocolMessage::File {
                    name,
                    size,
                    hash,
                    chunks,
                    timestamp,
                    expiry: expiry.unwrap_or(MESSAGE_EXPIRY_SECONDS),
                    ephemeral_id: ephemeral_id.unwrap_or_else(|| self.generate_ephemeral_id()),
                    delivery_id: delivery_id.clone(),
                }
            }
            ProtocolMessage::Control { command, data, ephemeral_id } => {
                ProtocolMessage::Control {
                    command,
                    data,
                    ephemeral_id: ephemeral_id.unwrap_or_else(|| self.generate_ephemeral_id()),
                }
            }
        };

        let serialized = serde_json::to_vec(&message)?;
        
        if serialized.len() > MAX_MESSAGE_SIZE {
            return Err(anyhow::anyhow!("Message too large"));
        }
        
        let encrypted = self.crypto_service.encrypt_message(&serialized)?;
        
        unimplemented!()
    }

    async fn send_delivery_confirmation(&mut self, peer_id: String, delivery_id: String) -> Result<()> {
        let confirmation = ProtocolMessage::Control {
            command: ControlCommand::DeliveryConfirmed(delivery_id),
            data: vec![],
            ephemeral_id: self.generate_ephemeral_id(),
        };
        
        self.send_message(peer_id, confirmation).await?;
        Ok(())
    }

    async fn handle_delivery_confirmation(&mut self, delivery_id: String) -> Result<()> {
        if let Some(info) = self.delivery_tracker.get_mut(&delivery_id) {
            info.is_confirmed = true;
            self.delivery_tracker.remove(&delivery_id);
        }
        Ok(())
    }

    async fn handle_delivery_failure(&mut self, delivery_id: String, reason: String) -> Result<()> {
        if let Some(info) = self.delivery_tracker.get_mut(&delivery_id) {
            info.retry_count += 1;
            if info.retry_count >= 3 {
                self.delivery_tracker.remove(&delivery_id);
            }
        }
        Ok(())
    }

    async fn handle_group_join(&mut self, peer_id: String, group_id: String) -> Result<()> {
        let group = self.group_members.entry(group_id.clone()).or_insert_with(|| GroupInfo {
            members: HashSet::new(),
            created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            last_activity: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            ephemeral_id: self.generate_ephemeral_id(),
        });

        if group.members.len() >= MAX_GROUP_SIZE {
            return Err(anyhow::anyhow!("Group is full"));
        }

        group.members.insert(peer_id.clone());
        group.last_activity = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        let notification = ProtocolMessage::Control {
            command: ControlCommand::GroupJoin(group_id),
            data: vec![],
            ephemeral_id: self.generate_ephemeral_id(),
        };

        for member in &group.members {
            if member != &peer_id {
                self.send_message(member.clone(), notification.clone()).await?;
            }
        }

        Ok(())
    }

    async fn handle_group_leave(&mut self, peer_id: String, group_id: String) -> Result<()> {
        if let Some(group) = self.group_members.get_mut(&group_id) {
            group.members.remove(&peer_id);
            group.last_activity = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

            let notification = ProtocolMessage::Control {
                command: ControlCommand::GroupLeave(group_id),
                data: vec![],
                ephemeral_id: self.generate_ephemeral_id(),
            };

            for member in &group.members {
                self.send_message(member.clone(), notification.clone()).await?;
            }

            if group.members.is_empty() {
                self.group_members.remove(&group_id);
            }
        }

        Ok(())
    }

    async fn handle_group_message_received(&mut self, group_id: String, message_id: String) -> Result<()> {
        if let Some(group) = self.group_members.get_mut(&group_id) {
            group.last_activity = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        }
        Ok(())
    }

    async fn handle_group_message_expired(&mut self, group_id: String, message_id: String) -> Result<()> {
        log::warn!("Received expired group message: {} in group {}", message_id, group_id);
        
        let notification = ProtocolMessage::Control {
            command: ControlCommand::GroupMessageExpired(group_id, message_id),
            data: vec![],
            ephemeral_id: self.generate_ephemeral_id(),
        };

        if let Some(group) = self.group_members.get(&group_id) {
            for member in &group.members {
                self.send_message(member.clone(), notification.clone()).await?;
            }
        }

        Ok(())
    }

    async fn handle_expired_group_message(&mut self, group_id: String, message_id: String) -> Result<()> {
        log::warn!("Received expired group message: {} in group {}", message_id, group_id);
        
        let notification = ProtocolMessage::Control {
            command: ControlCommand::GroupMessageExpired(group_id, message_id),
            data: vec![],
            ephemeral_id: self.generate_ephemeral_id(),
        };

        if let Some(group) = self.group_members.get(&group_id) {
            for member in &group.members {
                self.send_message(member.clone(), notification.clone()).await?;
            }
        }

        Ok(())
    }

    async fn handle_file_failure(&mut self, file_id: String, reason: String) -> Result<()> {
        let notification = ProtocolMessage::Control {
            command: ControlCommand::FileFailed(file_id.clone(), reason),
            data: vec![],
            ephemeral_id: self.generate_ephemeral_id(),
        };
        
        self.message_sender.send(notification).await?;
        self.file_transfers.remove(&file_id);
        Ok(())
    }

    async fn handle_chunk_received(&mut self, file_id: String, chunk_index: u32) -> Result<()> {
        if let Some(transfer) = self.file_transfers.get_mut(&file_id) {
            transfer.received_chunks.insert(chunk_index);
            transfer.last_update = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

            // Check if file is complete
            if transfer.received_chunks.len() as u32 == transfer.total_chunks {
                transfer.is_complete = true;
                let notification = ProtocolMessage::Control {
                    command: ControlCommand::FileComplete(file_id),
                    data: vec![],
                    ephemeral_id: self.generate_ephemeral_id(),
                };
                self.message_sender.send(notification).await?;
            }
        }
        Ok(())
    }

    async fn handle_chunk_missing(&mut self, file_id: String, chunk_index: u32) -> Result<()> {
        if let Some(transfer) = self.file_transfers.get_mut(&file_id) {
            transfer.retry_count += 1;
            if transfer.retry_count > MAX_CHUNK_RETRIES {
                self.handle_file_failure(file_id, "Max retries exceeded".to_string()).await?;
                return Ok(());
            }

            // Request missing chunk
            let chunk_request = ProtocolMessage::Control {
                command: ControlCommand::ChunkMissing(file_id, chunk_index),
                data: vec![],
                ephemeral_id: self.generate_ephemeral_id(),
            };
            self.message_sender.send(chunk_request).await?;
        }
        Ok(())
    }

    async fn process_pending_chunks(&mut self, file_id: String, peer_id: String) -> Result<()> {
        if let Some(transfer) = self.file_transfers.get_mut(&file_id) {
            if transfer.is_cancelled {
                return Ok(());
            }

            // Check bandwidth allocation
            let allocated = self.bandwidth_manager.allocate_bandwidth(&file_id, transfer.bandwidth_limit);
            if allocated == 0 {
                return Ok(());
            }

            // Adapt speed if needed
            self.adapt_transfer_speed(&file_id).await?;

            let mut chunks_to_send = Vec::new();
            let mut available_slots = MAX_PARALLEL_CHUNKS - transfer.active_chunks.len();
            let mut total_size = 0;

            // Collect chunks to send based on priority and resume point
            for &chunk_index in &transfer.pending_chunks {
                if chunk_index < transfer.resume_point {
                    continue;
                }
                if available_slots == 0 || total_size >= allocated {
                    break;
                }
                if !transfer.active_chunks.contains(&chunk_index) {
                    chunks_to_send.push(chunk_index);
                    transfer.active_chunks.insert(chunk_index);
                    available_slots -= 1;
                    total_size += CHUNK_SIZE as u64;
                }
            }

            // Send chunk requests
            if !chunks_to_send.is_empty() {
                let request = ProtocolMessage::FileChunkRequest {
                    file_id: file_id.clone(),
                    chunk_indices: chunks_to_send,
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                    expiry: MESSAGE_EXPIRY_SECONDS,
                    ephemeral_id: self.generate_ephemeral_id(),
                };
                self.send_message(peer_id, request).await?;
            }

            // Update progress
            self.update_transfer_progress(&file_id).await?;
        }
        Ok(())
    }

    async fn update_transfer_progress(&mut self, file_id: &str) -> Result<()> {
        if let Some(transfer) = self.file_transfers.get_mut(file_id) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;

            if now - transfer.last_progress_update >= PROGRESS_UPDATE_INTERVAL_MS {
                let total_bytes = transfer.total_chunks as u64 * CHUNK_SIZE as u64;
                let bytes_transferred = transfer.bytes_transferred;
                let elapsed = now - transfer.start_time;
                let speed = if elapsed > 0 {
                    (bytes_transferred as f64) / (elapsed as f64 / 1000.0)
                } else {
                    0.0
                };
                let estimated_time = if speed > 0.0 {
                    ((total_bytes - bytes_transferred) as f64 / speed) as u64
                } else {
                    0
                };

                let progress = ProtocolMessage::FileTransferProgress {
                    file_id: file_id.to_string(),
                    bytes_transferred,
                    total_bytes,
                    speed,
                    estimated_time,
                    timestamp: now,
                    expiry: MESSAGE_EXPIRY_SECONDS,
                    ephemeral_id: self.generate_ephemeral_id(),
                };
                self.message_sender.send(progress).await?;
            }
        }
        Ok(())
    }

    async fn adapt_transfer_speed(&mut self, file_id: &str) -> Result<()> {
        if let Some(transfer) = self.file_transfers.get_mut(file_id) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;

            if now - transfer.last_speed_adaptation >= SPEED_ADAPTATION_INTERVAL_MS {
                let speed_ratio = transfer.speed / transfer.target_speed;
                
                if speed_ratio < MIN_SPEED_THRESHOLD || speed_ratio > MAX_SPEED_THRESHOLD {
                    // Adjust bandwidth based on speed ratio
                    let new_bandwidth = (transfer.bandwidth_limit as f64 * speed_ratio) as u64;
                    let adjusted_bandwidth = new_bandwidth.clamp(
                        MIN_BANDWIDTH_BYTES_PER_SEC,
                        MAX_BANDWIDTH_BYTES_PER_SEC
                    );
                    
                    transfer.bandwidth_limit = adjusted_bandwidth;
                    self.bandwidth_manager.allocate_bandwidth(file_id, adjusted_bandwidth);
                    transfer.last_speed_adaptation = now;
                }
            }
        }
        Ok(())
    }

    async fn process_transfer_queue(&mut self) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        if now - self.transfer_queue.last_check >= QUEUE_CHECK_INTERVAL_MS {
            if let Some(file_id) = self.transfer_queue.next_transfer() {
                if let Some(transfer) = self.file_transfers.get(&file_id) {
                    // Update queue position for all transfers
                    for (id, pos) in self.transfer_queue.queue.iter().enumerate() {
                        let notification = ProtocolMessage::FileTransferQueueUpdate {
                            file_id: pos.clone(),
                            queue_position: id as u32,
                            timestamp: now,
                            expiry: MESSAGE_EXPIRY_SECONDS,
                            ephemeral_id: self.generate_ephemeral_id(),
                        };
                        self.message_sender.send(notification).await?;
                    }

                    // Process the next transfer
                    self.process_pending_chunks(file_id.clone(), "peer".to_string()).await?;
                }
            }
            self.transfer_queue.last_check = now;
        }
        Ok(())
    }

    pub async fn cleanup_expired_messages(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // Clean up expired messages
        let expired_ids: Vec<String> = self.message_tracker
            .iter()
            .filter(|(_, info)| now > info.timestamp + info.expiry)
            .map(|(id, _)| id.clone())
            .collect();
        
        for id in expired_ids {
            self.message_tracker.remove(&id);
        }

        // Clean up expired delivery confirmations
        let expired_deliveries: Vec<String> = self.delivery_tracker
            .iter()
            .filter(|(_, info)| now > info.timestamp + MESSAGE_EXPIRY_SECONDS)
            .map(|(id, _)| id.clone())
            .collect();
        
        for id in expired_deliveries {
            self.delivery_tracker.remove(&id);
        }

        // Clean up inactive groups
        let inactive_groups: Vec<String> = self.group_members
            .iter()
            .filter(|(_, info)| now > info.last_activity + MESSAGE_EXPIRY_SECONDS)
            .map(|(id, _)| id.clone())
            .collect();
        
        for id in inactive_groups {
            self.group_members.remove(&id);
        }

        // Clean up failed file transfers
        let failed_transfers: Vec<String> = self.file_transfers
            .iter()
            .filter(|(_, info)| info.error_count >= MAX_ERROR_RETRIES)
            .map(|(id, _)| id.clone())
            .collect();
        
        for id in failed_transfers {
            self.file_transfers.remove(&id);
            self.bandwidth_manager.release_bandwidth(&id);
            self.transfer_queue.remove_transfer(&id);
            self.transfer_priorities.retain(|p| p.file_id != id);
        }

        // Clean up rate limiter
        self.rate_limiter.cleanup();

        // Clean up cancelled transfers
        let cancelled_transfers: Vec<String> = self.file_transfers
            .iter()
            .filter(|(_, info)| info.is_cancelled)
            .map(|(id, _)| id.clone())
            .collect();
        
        for id in cancelled_transfers {
            self.file_transfers.remove(&id);
            self.bandwidth_manager.release_bandwidth(&id);
            self.transfer_queue.remove_transfer(&id);
            self.transfer_priorities.retain(|p| p.file_id != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::CryptoService;

    #[tokio::test]
    async fn test_protocol_handler_creation() {
        let crypto_service = CryptoService::new().unwrap();
        let handler = ProtocolHandler::new(crypto_service);
        assert!(handler.active_streams.is_empty());
    }

    #[tokio::test]
    async fn test_message_expiry() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let ephemeral_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        handler.track_message(ephemeral_id.clone(), timestamp, 1, None);
        assert!(handler.message_tracker.contains_key(&ephemeral_id));
        
        tokio::time::sleep(Duration::from_secs(2)).await;
        handler.cleanup_expired_messages().await;
        assert!(!handler.message_tracker.contains_key(&ephemeral_id));
    }

    #[tokio::test]
    async fn test_delivery_confirmation() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let ephemeral_id = handler.generate_ephemeral_id();
        let delivery_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        handler.track_message(ephemeral_id.clone(), timestamp, MESSAGE_EXPIRY_SECONDS, Some(delivery_id.clone()));
        assert!(handler.delivery_tracker.contains_key(&delivery_id));
        
        handler.handle_delivery_confirmation(delivery_id.clone()).await.unwrap();
        assert!(!handler.delivery_tracker.contains_key(&delivery_id));
    }

    #[tokio::test]
    async fn test_group_messaging() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let group_id = handler.generate_ephemeral_id();
        let peer1 = handler.generate_ephemeral_id();
        let peer2 = handler.generate_ephemeral_id();
        
        handler.handle_group_join(peer1.clone(), group_id.clone()).await.unwrap();
        handler.handle_group_join(peer2.clone(), group_id.clone()).await.unwrap();
        
        let group = handler.group_members.get(&group_id).unwrap();
        assert!(group.members.contains(&peer1));
        assert!(group.members.contains(&peer2));
        
        handler.handle_group_leave(peer1.clone(), group_id.clone()).await.unwrap();
        let group = handler.group_members.get(&group_id).unwrap();
        assert!(!group.members.contains(&peer1));
        assert!(group.members.contains(&peer2));
        
        handler.handle_group_leave(peer2.clone(), group_id.clone()).await.unwrap();
        assert!(!handler.group_members.contains_key(&group_id));
    }

    #[tokio::test]
    async fn test_file_transfer_progress() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // Initialize file transfer
        handler.file_transfers.insert(file_id.clone(), FileTransferInfo {
            file_id: file_id.clone(),
            total_chunks: 10,
            received_chunks: HashSet::new(),
            start_time: timestamp,
            last_update: timestamp,
            is_complete: false,
            retry_count: 0,
            pending_chunks: HashSet::from_iter(0..10),
            active_chunks: HashSet::new(),
            priority: 0,
            bandwidth_limit: 0,
            bytes_transferred: 0,
            last_progress_update: timestamp,
            speed: 0.0,
            is_cancelled: false,
            resume_point: 0,
            target_speed: 0.0,
            error_count: 0,
            last_error: None,
            last_speed_adaptation: timestamp,
        });

        // Test chunk reception
        handler.handle_chunk_received(file_id.clone(), 0).await.unwrap();
        handler.handle_chunk_received(file_id.clone(), 1).await.unwrap();
        
        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert!(transfer.received_chunks.contains(&0));
        assert!(transfer.received_chunks.contains(&1));
        assert!(!transfer.is_complete);

        // Test file completion
        for i in 2..10 {
            handler.handle_chunk_received(file_id.clone(), i).await.unwrap();
        }
        
        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert!(transfer.is_complete);
    }

    #[tokio::test]
    async fn test_chunk_resumption() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // Initialize file transfer
        handler.file_transfers.insert(file_id.clone(), FileTransferInfo {
            file_id: file_id.clone(),
            total_chunks: 10,
            received_chunks: HashSet::new(),
            start_time: timestamp,
            last_update: timestamp,
            is_complete: false,
            retry_count: 0,
            pending_chunks: HashSet::from_iter(0..10),
            active_chunks: HashSet::new(),
            priority: 0,
            bandwidth_limit: 0,
            bytes_transferred: 0,
            last_progress_update: timestamp,
            speed: 0.0,
            is_cancelled: false,
            resume_point: 0,
            target_speed: 0.0,
            error_count: 0,
            last_error: None,
            last_speed_adaptation: timestamp,
        });

        // Test chunk missing
        handler.handle_chunk_missing(file_id.clone(), 0).await.unwrap();
        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert_eq!(transfer.retry_count, 1);

        // Test max retries
        for _ in 0..MAX_CHUNK_RETRIES {
            handler.handle_chunk_missing(file_id.clone(), 0).await.unwrap();
        }
        
        assert!(!handler.file_transfers.contains_key(&file_id));
    }

    #[tokio::test]
    async fn test_parallel_chunk_transfer() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // Initialize file transfer
        handler.file_transfers.insert(file_id.clone(), FileTransferInfo {
            file_id: file_id.clone(),
            total_chunks: 10,
            received_chunks: HashSet::new(),
            start_time: timestamp,
            last_update: timestamp,
            is_complete: false,
            retry_count: 0,
            pending_chunks: HashSet::from_iter(0..10),
            active_chunks: HashSet::new(),
            priority: 0,
            bandwidth_limit: 0,
            bytes_transferred: 0,
            last_progress_update: timestamp,
            speed: 0.0,
            is_cancelled: false,
            resume_point: 0,
            target_speed: 0.0,
            error_count: 0,
            last_error: None,
            last_speed_adaptation: timestamp,
        });

        // Test parallel chunk processing
        handler.process_pending_chunks(file_id.clone(), "peer1".to_string()).await.unwrap();
        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert_eq!(transfer.active_chunks.len(), MAX_PARALLEL_CHUNKS);
        assert_eq!(transfer.pending_chunks.len(), 10 - MAX_PARALLEL_CHUNKS);
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Test rate limiting
        for _ in 0..RATE_LIMIT_CHUNKS_PER_SEC {
            assert!(handler.rate_limiter.can_send_chunks(file_id));
        }
        assert!(!handler.rate_limiter.can_send_chunks(file_id));

        // Test rate limit cleanup
        tokio::time::sleep(Duration::from_millis(RATE_LIMIT_WINDOW_MS as u64 + 100)).await;
        handler.rate_limiter.cleanup();
        assert!(handler.rate_limiter.can_send_chunks(file_id));
    }

    #[tokio::test]
    async fn test_file_transfer_priority() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Set file transfer priority
        let priority_msg = ProtocolMessage::FileTransferPriority {
            file_id: file_id.clone(),
            priority: 3,
            timestamp,
            expiry: MESSAGE_EXPIRY_SECONDS,
            ephemeral_id: handler.generate_ephemeral_id(),
        };
        handler.handle_message("peer1".to_string(), serde_json::to_vec(&priority_msg).unwrap()).await.unwrap();

        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert_eq!(transfer.priority, 3);
    }

    #[tokio::test]
    async fn test_file_transfer_cancellation() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Cancel file transfer
        let cancel_msg = ProtocolMessage::FileTransferCancel {
            file_id: file_id.clone(),
            reason: "User cancelled".to_string(),
            timestamp,
            expiry: MESSAGE_EXPIRY_SECONDS,
            ephemeral_id: handler.generate_ephemeral_id(),
        };
        handler.handle_message("peer1".to_string(), serde_json::to_vec(&cancel_msg).unwrap()).await.unwrap();

        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert!(transfer.is_cancelled);
    }

    #[tokio::test]
    async fn test_bandwidth_throttling() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Set bandwidth limit
        let bandwidth_msg = ProtocolMessage::FileTransferBandwidth {
            file_id: file_id.clone(),
            bandwidth_limit: 1024 * 1024, // 1MB/s
            timestamp,
            expiry: MESSAGE_EXPIRY_SECONDS,
            ephemeral_id: handler.generate_ephemeral_id(),
        };
        handler.handle_message("peer1".to_string(), serde_json::to_vec(&bandwidth_msg).unwrap()).await.unwrap();

        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert_eq!(transfer.bandwidth_limit, 1024 * 1024);
    }

    #[tokio::test]
    async fn test_progress_estimation() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Update progress
        let progress_msg = ProtocolMessage::FileTransferProgress {
            file_id: file_id.clone(),
            bytes_transferred: 1024 * 1024, // 1MB
            total_bytes: 2 * 1024 * 1024, // 2MB
            speed: 1024.0, // 1KB/s
            estimated_time: 1024, // 1 second
            timestamp,
            expiry: MESSAGE_EXPIRY_SECONDS,
            ephemeral_id: handler.generate_ephemeral_id(),
        };
        handler.handle_message("peer1".to_string(), serde_json::to_vec(&progress_msg).unwrap()).await.unwrap();

        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert_eq!(transfer.bytes_transferred, 1024 * 1024);
        assert_eq!(transfer.speed, 1024.0);
    }

    #[tokio::test]
    async fn test_transfer_resumption() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Resume transfer
        let resume_msg = ProtocolMessage::FileTransferResume {
            file_id: file_id.clone(),
            resume_point: 5,
            timestamp,
            expiry: MESSAGE_EXPIRY_SECONDS,
            ephemeral_id: handler.generate_ephemeral_id(),
        };
        handler.handle_message("peer1".to_string(), serde_json::to_vec(&resume_msg).unwrap()).await.unwrap();

        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert_eq!(transfer.resume_point, 5);
        assert_eq!(transfer.error_count, 0);
    }

    #[tokio::test]
    async fn test_speed_adaptation() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Set target speed
        let speed_msg = ProtocolMessage::FileTransferSpeedAdapt {
            file_id: file_id.clone(),
            target_speed: 1024.0, // 1KB/s
            timestamp,
            expiry: MESSAGE_EXPIRY_SECONDS,
            ephemeral_id: handler.generate_ephemeral_id(),
        };
        handler.handle_message("peer1".to_string(), serde_json::to_vec(&speed_msg).unwrap()).await.unwrap();

        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert_eq!(transfer.target_speed, 1024.0);
    }

    #[tokio::test]
    async fn test_transfer_queue() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id1 = handler.generate_ephemeral_id();
        let file_id2 = handler.generate_ephemeral_id();
        
        // Add transfers to queue
        handler.transfer_queue.add_transfer(file_id1.clone());
        handler.transfer_queue.add_transfer(file_id2.clone());
        
        assert_eq!(handler.transfer_queue.get_position(&file_id1), Some(0));
        assert_eq!(handler.transfer_queue.get_position(&file_id2), Some(1));
        
        // Remove transfer
        handler.transfer_queue.remove_transfer(&file_id1);
        assert_eq!(handler.transfer_queue.get_position(&file_id2), Some(0));
    }

    #[tokio::test]
    async fn test_error_recovery() {
        let crypto_service = CryptoService::new().unwrap();
        let mut handler = ProtocolHandler::new(crypto_service);
        
        let file_id = handler.generate_ephemeral_id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Report error
        let error_msg = ProtocolMessage::FileTransferError {
            file_id: file_id.clone(),
            error_type: "Network error".to_string(),
            retry_count: 1,
            timestamp,
            expiry: MESSAGE_EXPIRY_SECONDS,
            ephemeral_id: handler.generate_ephemeral_id(),
        };
        handler.handle_message("peer1".to_string(), serde_json::to_vec(&error_msg).unwrap()).await.unwrap();

        let transfer = handler.file_transfers.get(&file_id).unwrap();
        assert_eq!(transfer.error_count, 1);
        assert_eq!(transfer.last_error, Some("Network error".to_string()));
    }
} 