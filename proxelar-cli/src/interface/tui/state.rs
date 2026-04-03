use std::collections::{HashMap, VecDeque};

use proxyapi::ProxyEvent;
use ratatui::widgets::TableState;

/// Maximum number of stored requests before old entries are evicted.
const MAX_REQUESTS: usize = 10_000;

/// Maximum accumulated streaming body size per event (10 MB).
const MAX_STREAMING_BODY: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Request,
    Response,
}

#[derive(Debug)]
pub enum PendingAction {
    OpenEditor,
}

pub struct AppState {
    pub(crate) requests: VecDeque<ProxyEvent>,
    /// Accumulated streaming response body data, keyed by event ID.
    pub(crate) streaming_bodies: HashMap<u64, Vec<u8>>,
    pub table_state: TableState,
    pub detail_open: bool,
    pub detail_tab: DetailTab,
    pub detail_scroll: u16,
    pub pending_action: Option<PendingAction>,
    pub filter: Option<String>,
    pub filter_input: String,
    pub filter_mode: bool,
}

/// Returns `true` if `event` matches the given filter string (case-insensitive).
pub(crate) fn matches_filter(event: &ProxyEvent, filter: Option<&str>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let filter_lower = filter.to_lowercase();
    match event {
        ProxyEvent::RequestComplete { request, .. } => {
            let uri = request.uri().to_string();
            let method = request.method().as_str();
            uri.to_lowercase().contains(&filter_lower)
                || method.to_lowercase().contains(&filter_lower)
        }
        ProxyEvent::Error { message } => message.to_lowercase().contains(&filter_lower),
        ProxyEvent::StreamingChunk { .. } => false,
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            requests: VecDeque::new(),
            streaming_bodies: HashMap::new(),
            table_state: TableState::default(),
            detail_open: false,
            detail_tab: DetailTab::Request,
            detail_scroll: 0,
            pending_action: None,
            filter: None,
            filter_input: String::new(),
            filter_mode: false,
        }
    }

    pub fn add_event(&mut self, event: ProxyEvent) {
        self.requests.push_back(event);
        if self.requests.len() > MAX_REQUESTS {
            if let Some(ProxyEvent::RequestComplete { id, .. }) = self.requests.pop_front().as_ref()
            {
                self.streaming_bodies.remove(id);
            }
            if let Some(idx) = self.table_state.selected() {
                self.table_state.select(Some(idx.saturating_sub(1)));
            }
        }
    }

    /// Append streaming chunk data to the accumulated buffer for the given event ID.
    pub fn append_streaming_data(&mut self, id: u64, data: &[u8]) {
        let buf = self.streaming_bodies.entry(id).or_default();
        let remaining = MAX_STREAMING_BODY.saturating_sub(buf.len());
        if remaining > 0 {
            let to_copy = data.len().min(remaining);
            buf.extend_from_slice(&data[..to_copy]);
        }
    }

    fn filtered_count(&self) -> usize {
        let filter = self.filter.as_deref();
        self.requests
            .iter()
            .filter(|event| matches_filter(event, filter))
            .count()
    }

    pub fn select_next(&mut self) {
        let len = self.filtered_count();
        if len == 0 {
            return;
        }
        let i = self
            .table_state
            .selected()
            .map_or(0, |i| (i + 1).min(len - 1));
        self.table_state.select(Some(i));
        self.detail_scroll = 0;
    }

    pub fn select_prev(&mut self) {
        let i = self
            .table_state
            .selected()
            .map_or(0, |i| i.saturating_sub(1));
        self.table_state.select(Some(i));
        self.detail_scroll = 0;
    }

    pub fn select_first(&mut self) {
        if self.filtered_count() > 0 {
            self.table_state.select(Some(0));
            self.detail_scroll = 0;
        }
    }

    pub fn select_last(&mut self) {
        let len = self.filtered_count();
        if len > 0 {
            self.table_state.select(Some(len - 1));
            self.detail_scroll = 0;
        }
    }

    pub fn toggle_detail(&mut self) {
        self.detail_open = !self.detail_open;
        self.detail_scroll = 0;
    }

    pub fn toggle_tab(&mut self) {
        self.detail_tab = match self.detail_tab {
            DetailTab::Request => DetailTab::Response,
            DetailTab::Response => DetailTab::Request,
        };
        self.detail_scroll = 0;
    }

    pub fn scroll_detail_down(&mut self, step: u16) {
        self.detail_scroll = self.detail_scroll.saturating_add(step);
    }

    pub fn scroll_detail_up(&mut self, step: u16) {
        self.detail_scroll = self.detail_scroll.saturating_sub(step);
    }

    /// Returns a reference to the currently selected event (respecting the active filter).
    pub fn selected_event(&self) -> Option<&ProxyEvent> {
        let filter = self.filter.as_deref();
        let selected = self.table_state.selected()?;
        self.requests
            .iter()
            .filter(|e| matches_filter(e, filter))
            .nth(selected)
    }

    pub fn clear(&mut self) {
        self.requests.clear();
        self.streaming_bodies.clear();
        self.table_state.select(None);
        self.detail_open = false;
        self.detail_scroll = 0;
    }
}
