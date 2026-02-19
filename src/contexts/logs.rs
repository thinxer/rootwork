use crate::contexts::Context;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

#[link(name = "systemd")]
unsafe extern "C" {
    fn sd_journal_open(ret: *mut *mut c_void, flags: c_int) -> c_int;
    fn sd_journal_close(j: *mut c_void);
    fn sd_journal_add_match(j: *mut c_void, data: *const c_void, size: usize) -> c_int;
    fn sd_journal_seek_tail(j: *mut c_void) -> c_int;
    fn sd_journal_seek_realtime_usec(j: *mut c_void, usec: u64) -> c_int;
    fn sd_journal_previous(j: *mut c_void) -> c_int;
    fn sd_journal_next(j: *mut c_void) -> c_int;
    fn sd_journal_get_realtime_usec(j: *mut c_void, ret: *mut u64) -> c_int;
    fn sd_journal_get_data(
        j: *mut c_void,
        field: *const c_char,
        data: *mut *const u8,
        length: *mut usize,
    ) -> c_int;
}

const SD_JOURNAL_LOCAL_ONLY: c_int = 1;

pub struct LogEntry {
    timestamp_micros: u64,
    display_time: String,
    unit: String,
    message: String,
    priority: u8,
}

pub struct LogsContext {
    entries: VecDeque<LogEntry>,
    max_entries: usize,
    filter_unit: Option<String>,
    paused: bool,
    follow_mode: bool,
    selected: usize,
}

impl LogsContext {
    pub fn new() -> Self {
        let mut ctx = Self {
            entries: VecDeque::new(),
            max_entries: 1000,
            filter_unit: None,
            paused: false,
            follow_mode: true,
            selected: 0,
        };
        ctx.load_entries();
        ctx
    }

    fn load_entries(&mut self) {
        self.entries.clear();
        self.selected = 0;

        let fresh = JournalReader::read_recent(self.filter_unit.as_deref(), 100);
        for e in fresh {
            self.add_entry(e);
        }

        if self.follow_mode {
            self.scroll_to_bottom();
        }
    }

    pub fn refresh(&mut self) {
        if self.paused {
            return;
        }

        let last_seen = self.entries.back().map(|e| e.timestamp_micros).unwrap_or(0);
        let old_len = self.entries.len();

        let fresh = JournalReader::read_since(self.filter_unit.as_deref(), last_seen);
        for e in fresh {
            self.add_entry(e);
        }

        if self.follow_mode && !self.paused && self.entries.len() > old_len {
            self.scroll_to_bottom();
        }
    }

    fn add_entry(&mut self, entry: LogEntry) {
        self.entries.push_back(entry);
        if self.entries.len() > self.max_entries {
            self.entries.pop_front();
            if self.selected > 0 {
                self.selected -= 1;
            }
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.follow_mode = false;
        }
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
            if self.selected == self.entries.len() - 1 {
                self.follow_mode = true;
            }
        }
    }

    fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(10);
        self.follow_mode = false;
    }

    fn page_down(&mut self) {
        self.selected = (self.selected + 10).min(self.entries.len().saturating_sub(1));
        if self.selected == self.entries.len().saturating_sub(1) {
            self.follow_mode = true;
        }
    }

    fn scroll_to_bottom(&mut self) {
        if !self.entries.is_empty() {
            self.selected = self.entries.len() - 1;
        }
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    fn toggle_follow(&mut self) {
        self.follow_mode = !self.follow_mode;
        if self.follow_mode {
            self.scroll_to_bottom();
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.selected = 0;
    }
}

impl Context for LogsContext {
    fn name(&self) -> &'static str {
        "Logs"
    }

    fn draw(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(format!(
                " Journal Logs {}{}{} ",
                if self.paused { "[PAUSED] " } else { "" },
                if self.follow_mode { "[follow] " } else { "" },
                self.filter_unit
                    .as_ref()
                    .map(|u| format!("[{}] ", u))
                    .unwrap_or_default()
            ))
            .borders(Borders::ALL);

        let visible_lines = area.height.saturating_sub(2) as usize;
        if visible_lines == 0 {
            f.render_widget(Paragraph::new("").block(block), area);
            return;
        }

        let scroll_offset = if self.entries.len() <= visible_lines {
            0
        } else if self.selected >= self.entries.len().saturating_sub(visible_lines) {
            self.entries.len().saturating_sub(visible_lines)
        } else {
            self.selected
        };

        let lines: Vec<Line> = self
            .entries
            .iter()
            .skip(scroll_offset)
            .take(visible_lines)
            .enumerate()
            .map(|(i, entry)| {
                let actual_idx = scroll_offset + i;
                let is_selected = actual_idx == self.selected;
                let bg_style = if is_selected {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };

                let priority_color = match entry.priority {
                    0..=2 => Color::Red,
                    3 => Color::LightRed,
                    4 => Color::Yellow,
                    5 => Color::Green,
                    6 => Color::Blue,
                    _ => Color::Gray,
                };

                let msg = if entry.message.len() > 200 {
                    format!("{}...", &entry.message[..200])
                } else {
                    entry.message.clone()
                };

                Line::from(vec![
                    Span::styled(
                        format!("{:15} ", entry.display_time),
                        Style::default().fg(Color::Gray),
                    ),
                    Span::styled(
                        format!("{:20} ", &entry.unit[..entry.unit.len().min(20)]),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(msg, Style::default().fg(priority_color)),
                ])
                .style(bg_style)
            })
            .collect();

        if lines.is_empty() {
            f.render_widget(Paragraph::new("No log entries").block(block), area);
        } else {
            f.render_widget(Paragraph::new(lines).block(block), area);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Char(' ') | KeyCode::PageDown => self.page_down(),
            KeyCode::Char('b') | KeyCode::PageUp => self.page_up(),
            KeyCode::Char('G') => {
                self.scroll_to_bottom();
                self.follow_mode = true;
            }
            KeyCode::Char('g') => {
                self.selected = 0;
                self.follow_mode = false;
            }
            KeyCode::Char('p') => self.toggle_pause(),
            KeyCode::Char('f') => self.toggle_follow(),
            KeyCode::Char('c') => self.clear(),
            KeyCode::Char('r') => self.load_entries(),
            _ => {}
        }
    }

    async fn tick(&mut self) {
        self.refresh();
    }
}

struct JournalReader;

impl JournalReader {
    fn read_recent(unit: Option<&str>, max: usize) -> Vec<LogEntry> {
        let mut out = Vec::new();
        unsafe {
            let mut j: *mut c_void = std::ptr::null_mut();
            if sd_journal_open(&mut j as *mut *mut c_void, SD_JOURNAL_LOCAL_ONLY) < 0 || j.is_null() {
                return out;
            }

            if let Some(u) = unit {
                let m = format!("_SYSTEMD_UNIT={u}");
                let _ = sd_journal_add_match(j, m.as_ptr() as *const c_void, m.len());
            }

            let _ = sd_journal_seek_tail(j);
            for _ in 0..max {
                if sd_journal_previous(j) <= 0 {
                    break;
                }
                if let Some(e) = read_current_entry(j) {
                    out.push(e);
                }
            }
            sd_journal_close(j);
        }
        out.reverse();
        out
    }

    fn read_since(unit: Option<&str>, since_micros: u64) -> Vec<LogEntry> {
        let mut out = Vec::new();
        unsafe {
            let mut j: *mut c_void = std::ptr::null_mut();
            if sd_journal_open(&mut j as *mut *mut c_void, SD_JOURNAL_LOCAL_ONLY) < 0 || j.is_null() {
                return out;
            }

            if let Some(u) = unit {
                let m = format!("_SYSTEMD_UNIT={u}");
                let _ = sd_journal_add_match(j, m.as_ptr() as *const c_void, m.len());
            }

            let _ = sd_journal_seek_realtime_usec(j, since_micros.saturating_add(1));
            loop {
                if sd_journal_next(j) <= 0 {
                    break;
                }
                if let Some(e) = read_current_entry(j)
                    && e.timestamp_micros > since_micros
                {
                    out.push(e);
                }
                if out.len() >= 500 {
                    break;
                }
            }

            sd_journal_close(j);
        }
        out
    }
}

fn read_current_entry(j: *mut c_void) -> Option<LogEntry> {
    let timestamp_micros = get_realtime_usec(j)?;
    let message = get_field(j, "MESSAGE")?;
    let unit = get_field(j, "_SYSTEMD_UNIT")
        .or_else(|| get_field(j, "SYSLOG_IDENTIFIER"))
        .unwrap_or_else(|| "system".to_string());
    let priority = get_field(j, "PRIORITY")
        .and_then(|p| p.parse().ok())
        .unwrap_or(6);

    let ts_secs = (timestamp_micros / 1_000_000) as i64;
    let display_time = chrono::DateTime::from_timestamp(ts_secs, 0)
        .map(|dt| {
            let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(dt);
            local.format("%y%m%d %H:%M:%S").to_string()
        })
        .unwrap_or_else(|| "?".to_string());

    Some(LogEntry {
        timestamp_micros,
        display_time,
        unit,
        message,
        priority,
    })
}

fn get_realtime_usec(j: *mut c_void) -> Option<u64> {
    let mut ts = 0u64;
    let rc = unsafe { sd_journal_get_realtime_usec(j, &mut ts as *mut u64) };
    if rc >= 0 {
        Some(ts)
    } else {
        None
    }
}

fn get_field(j: *mut c_void, field: &str) -> Option<String> {
    let field_c = CString::new(field).ok()?;
    let mut data_ptr: *const u8 = std::ptr::null();
    let mut len: usize = 0;
    let rc = unsafe {
        sd_journal_get_data(
            j,
            field_c.as_ptr(),
            &mut data_ptr as *mut *const u8,
            &mut len as *mut usize,
        )
    };
    if rc < 0 || data_ptr.is_null() || len == 0 {
        return None;
    }

    let bytes = unsafe { std::slice::from_raw_parts(data_ptr, len) };
    let text = String::from_utf8_lossy(bytes);
    let prefix = format!("{}=", field);
    text.strip_prefix(&prefix).map(|s| s.to_string())
}
