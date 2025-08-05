use anyhow::Result;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::syntax::{HighlightCacheEntry, HighlightCacheKey, HighlightRange, SyntaxHighlighter};

#[cfg(test)]
mod tests;

/// Priority levels for syntax highlighting requests
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low = 0,      // Background processing of entire file
    Medium = 1,   // Lines within scroll buffer (±50 lines)
    High = 2,     // Currently visible lines
    Critical = 3, // User is actively editing this line
}

/// Request for syntax highlighting
#[derive(Debug)]
pub struct HighlightRequest {
    pub buffer_id: usize,
    pub line_index: usize,
    pub content: String,
    pub language: String,
    pub priority: Priority,
    pub response_tx: oneshot::Sender<Vec<HighlightRange>>,
}

/// Async syntax highlighter that processes requests in background
pub struct AsyncSyntaxHighlighter {
    /// Request sender to background worker
    request_tx: mpsc::UnboundedSender<HighlightRequest>,
    /// Handle to the background worker task
    worker_handle: JoinHandle<()>,
    /// Shared cache accessible from main thread for immediate lookups
    shared_cache: Arc<RwLock<HashMap<HighlightCacheKey, HighlightCacheEntry>>>,
}

impl AsyncSyntaxHighlighter {
    /// Create a new async syntax highlighter with background worker
    pub fn new() -> Result<Self> {
        info!("Initializing async syntax highlighter");

        // Check if we have a Tokio runtime available
        match tokio::runtime::Handle::try_current() {
            Ok(_) => {
                // We have a runtime, proceed with async initialization
                Self::new_with_runtime()
            }
            Err(_) => {
                // No runtime available, return error instead of panicking
                Err(anyhow::anyhow!(
                    "No Tokio runtime available for async syntax highlighter"
                ))
            }
        }
    }

    /// Internal method that assumes a Tokio runtime is available
    fn new_with_runtime() -> Result<Self> {
        // Create shared cache that both main thread and worker can access
        let shared_cache = Arc::new(RwLock::new(HashMap::new()));
        let worker_cache = Arc::clone(&shared_cache);

        // Create communication channel
        let (request_tx, request_rx) = mpsc::unbounded_channel();

        // Spawn background worker
        let worker_handle = tokio::spawn(async move {
            Self::worker_loop(request_rx, worker_cache).await;
        });

        Ok(AsyncSyntaxHighlighter {
            request_tx,
            worker_handle,
            shared_cache,
        })
    }

    /// Check if we have cached highlights for this line
    pub fn get_cached_highlights(
        &self,
        buffer_id: usize,
        line_index: usize,
        content: &str,
        language: &str,
    ) -> Option<Vec<HighlightRange>> {
        let cache_key = HighlightCacheKey::new_simple(content, language);

        if let Ok(cache) = self.shared_cache.read() {
            if let Some(entry) = cache.get(&cache_key) {
                debug!("Cache hit for buffer {} line {}", buffer_id, line_index);
                return Some(entry.highlights().clone());
            }
        }

        None
    }

    /// Get immediate synchronous highlighting for a line (for initial rendering)
    /// This bypasses the async queue and highlights immediately
    pub fn get_immediate_highlights(
        &self,
        buffer_id: usize,
        line_index: usize,
        content: &str,
        language: &str,
    ) -> Option<Vec<HighlightRange>> {
        // First check cache
        if let Some(cached) = self.get_cached_highlights(buffer_id, line_index, content, language) {
            return Some(cached);
        }

        // If not cached, create a temporary synchronous highlighter for immediate use
        if let Ok(mut sync_highlighter) = SyntaxHighlighter::new() {
            if let Ok(highlights) = sync_highlighter.highlight_text(content, language) {
                // Store in cache for future use
                let cache_key = HighlightCacheKey::new_simple(content, language);
                let cache_entry = HighlightCacheEntry::new(highlights.clone());

                if let Ok(mut cache) = self.shared_cache.write() {
                    cache.insert(cache_key, cache_entry);
                }

                return Some(highlights);
            }
        }

        None
    }

    /// Request syntax highlighting for a line (async)
    pub fn request_highlighting(
        &self,
        buffer_id: usize,
        line_index: usize,
        content: String,
        language: String,
        priority: Priority,
    ) -> Result<oneshot::Receiver<Vec<HighlightRange>>> {
        let (response_tx, response_rx) = oneshot::channel();

        let request = HighlightRequest {
            buffer_id,
            line_index,
            content,
            language,
            priority,
            response_tx,
        };

        self.request_tx.send(request).map_err(|_| {
            anyhow::anyhow!("Failed to send highlight request - worker may be shut down")
        })?;

        debug!(
            "Requested highlighting for buffer {} line {} with priority {:?}",
            buffer_id, line_index, priority
        );
        Ok(response_rx)
    }

    /// Request highlighting for multiple lines with priority
    pub fn request_batch_highlighting(
        &self,
        buffer_id: usize,
        lines: Vec<(usize, String)>, // (line_index, content)
        language: String,
        priority: Priority,
    ) -> Result<Vec<oneshot::Receiver<Vec<HighlightRange>>>> {
        let mut receivers = Vec::new();

        for (line_index, content) in lines {
            let receiver = self.request_highlighting(
                buffer_id,
                line_index,
                content,
                language.clone(),
                priority,
            )?;
            receivers.push(receiver);
        }

        Ok(receivers)
    }

    /// Invalidate cache entries for a buffer (when buffer is edited)
    pub fn invalidate_buffer_cache(&self, buffer_id: usize) {
        // For now, we'll do a simple approach and clear the entire cache
        // In a more sophisticated implementation, we could track which cache entries
        // belong to which buffer and only invalidate those
        if let Ok(mut cache) = self.shared_cache.write() {
            let before_size = cache.len();
            cache.clear();
            debug!(
                "Invalidated cache for buffer {} (cleared {} entries)",
                buffer_id, before_size
            );
        }
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> (usize, usize) {
        if let Ok(cache) = self.shared_cache.read() {
            (cache.len(), 1000) // (current_size, max_size)
        } else {
            (0, 1000)
        }
    }

    /// Background worker loop that processes highlighting requests
    async fn worker_loop(
        mut request_rx: mpsc::UnboundedReceiver<HighlightRequest>,
        cache: Arc<RwLock<HashMap<HighlightCacheKey, HighlightCacheEntry>>>,
    ) {
        info!("Starting async syntax highlighting worker");

        // Create a syntax highlighter for the worker thread
        let mut highlighter = match SyntaxHighlighter::new() {
            Ok(h) => h,
            Err(e) => {
                warn!("Failed to create syntax highlighter in worker: {}", e);
                return;
            }
        };

        // Use a priority queue to process high priority requests first
        let mut pending_requests: Vec<HighlightRequest> = Vec::new();

        while let Some(request) = request_rx.recv().await {
            // Add request to pending queue
            pending_requests.push(request);

            // Sort by priority (highest first)
            pending_requests.sort_by(|a, b| b.priority.cmp(&a.priority));

            // Process all pending requests in priority order
            while let Some(request) = pending_requests.pop() {
                Self::process_request(request, &mut highlighter, &cache).await;

                // Check if we have more incoming requests to potentially interrupt lower priority work
                if pending_requests.len() < 10 {
                    // Don't interrupt if we have a big backlog
                    if let Ok(new_request) = request_rx.try_recv() {
                        pending_requests.push(new_request);
                        pending_requests.sort_by(|a, b| b.priority.cmp(&a.priority));
                    }
                }
            }
        }

        info!("Async syntax highlighting worker stopped");
    }

    /// Process a single highlighting request
    async fn process_request(
        request: HighlightRequest,
        highlighter: &mut SyntaxHighlighter,
        cache: &Arc<RwLock<HashMap<HighlightCacheKey, HighlightCacheEntry>>>,
    ) {
        let cache_key = HighlightCacheKey::new_simple(&request.content, &request.language);

        // Check cache first
        if let Ok(cache_ref) = cache.read() {
            if let Some(entry) = cache_ref.get(&cache_key) {
                debug!(
                    "Worker cache hit for buffer {} line {}",
                    request.buffer_id, request.line_index
                );
                let _ = request.response_tx.send(entry.highlights().clone());
                return;
            }
        }

        // Not in cache, compute highlights
        let highlights = highlighter.highlight_line(&request.content, &request.language);

        // Convert to HighlightRange format
        let highlight_ranges: Vec<HighlightRange> = highlights
            .into_iter()
            .map(|(start, end, highlight_type)| {
                HighlightRange {
                    start,
                    end,
                    style: highlight_type.into(), // Convert to HighlightStyle
                }
            })
            .collect();

        debug!(
            "Worker computed highlights for buffer {} line {} ({} ranges)",
            request.buffer_id,
            request.line_index,
            highlight_ranges.len()
        );

        // Store in cache
        if let Ok(mut cache_ref) = cache.write() {
            let entry = HighlightCacheEntry::new(highlight_ranges.clone());
            cache_ref.insert(cache_key, entry);

            // Simple LRU: if cache is too big, clear it
            // In a production system, we'd implement proper LRU eviction
            if cache_ref.len() > 1000 {
                debug!("Cache full, clearing to prevent memory growth");
                cache_ref.clear();
            }
        }

        // Send result
        let _ = request.response_tx.send(highlight_ranges);
    }
}

impl Drop for AsyncSyntaxHighlighter {
    fn drop(&mut self) {
        // Abort the worker when the highlighter is dropped
        self.worker_handle.abort();
    }
}
