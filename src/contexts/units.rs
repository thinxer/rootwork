use crate::contexts::Context;
use crate::systemd::client::{SystemdClient, UnitInfo};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table},
    Frame,
};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

/// A log entry with timestamp for display
#[derive(Clone)]
pub struct UnitLogEntry {
    pub timestamp_micros: u64,
    pub display_time: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewMode {
    List,
    Tree,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortBy {
    Name,
    State,
}

/// An item in the tree view - either a group or a unit
#[derive(Debug, Clone)]
pub enum TreeItem {
    Group { name: String, count: usize, active: usize },
    Unit { unit: UnitInfo },
}

#[derive(Debug, Clone, Copy)]
enum UnitAction {
    Start,
    Stop,
    Enable,
    Disable,
}

impl UnitAction {
    fn label(&self) -> &'static str {
        match self {
            UnitAction::Start => "start",
            UnitAction::Stop => "stop",
            UnitAction::Enable => "enable",
            UnitAction::Disable => "disable",
        }
    }
}

unsafe extern "C" {
    fn sd_journal_open(ret: *mut *mut c_void, flags: c_int) -> c_int;
    fn sd_journal_close(j: *mut c_void);
    fn sd_journal_add_match(j: *mut c_void, data: *const c_void, size: usize) -> c_int;
    fn sd_journal_seek_tail(j: *mut c_void) -> c_int;
    fn sd_journal_previous(j: *mut c_void) -> c_int;
    fn sd_journal_get_realtime_usec(j: *mut c_void, ret: *mut u64) -> c_int;
    fn sd_journal_get_data(
        j: *mut c_void,
        field: *const c_char,
        data: *mut *const u8,
        length: *mut usize,
    ) -> c_int;
}

const SD_JOURNAL_LOCAL_ONLY: c_int = 1;

pub struct UnitsContext {
    units: Vec<UnitInfo>,
    filtered_units: Vec<UnitInfo>,
    tree_items: Vec<TreeItem>,
    selected: usize,
    scroll_offset: usize,
    filter: String,
    filter_backup: Option<String>,
    show_filter: bool,
    loading: bool,
    error: Option<String>,
    view_mode: ViewMode,
    sort_by: SortBy,
    sort_ascending: bool,
    collapsed_groups: HashSet<String>, // Set of collapsed group names
    systemd: SystemdClient,
    detail_unit: Option<UnitInfo>,
    detail_logs: Vec<UnitLogEntry>,
    confirm_action: Option<UnitAction>,
    pending_action: Option<UnitAction>,
    action_status: Option<String>,
    detail_log_scroll: usize,
    detail_log_follow: bool,
}

impl UnitsContext {
    pub async fn new(systemd: &SystemdClient) -> Result<Self> {
        let mut ctx = Self {
            units: Vec::new(),
            filtered_units: Vec::new(),
            tree_items: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            filter: String::new(),
            filter_backup: None,
            show_filter: false,
            loading: true,
            error: None,
            view_mode: ViewMode::Tree, // Default to tree view
            sort_by: SortBy::Name,
            sort_ascending: true,
            collapsed_groups: HashSet::new(), // Start with all collapsed
            systemd: systemd.clone(),
            detail_unit: None,
            detail_logs: Vec::new(),
            confirm_action: None,
            pending_action: None,
            action_status: None,
            detail_log_scroll: 0,
            detail_log_follow: true,
        };

        ctx.refresh(systemd).await;
        Ok(ctx)
    }

    pub async fn refresh(&mut self,
        systemd: &SystemdClient,
    ) {
        self.loading = true;
        self.error = None;

        match systemd.list_units().await {
            Ok(units) => {
                self.units = units;
                self.apply_filter_and_sort();
                self.loading = false;
            }
            Err(e) => {
                self.error = Some(format!("Failed to list units: {}", e));
                self.loading = false;
            }
        }
    }

    fn apply_filter_and_sort(&mut self,
    ) {
        // Filter + fuzzy ranking
        let mut ranked_units: Vec<(UnitInfo, Option<usize>)> = if self.filter.is_empty() {
            self.units
                .iter()
                .cloned()
                .map(|u| (u, None))
                .collect()
        } else {
            let needle = self.filter.trim().to_lowercase();
            self.units
                .iter()
                .filter_map(|u| {
                    let name = u.name.to_lowercase();
                    let desc = u.description.to_lowercase();

                    let name_score = fuzzy_match_score(&name, &needle);
                    let desc_score = fuzzy_match_score(&desc, &needle).map(|s| s + 200);

                    let best_score = match (name_score, desc_score) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (Some(a), None) => Some(a),
                        (None, Some(b)) => Some(b),
                        (None, None) => None,
                    };

                    best_score.map(|score| (u.clone(), Some(score)))
                })
                .collect()
        };

        // Sort
        ranked_units.sort_by(|(a, a_score), (b, b_score)| {
            let fuzzy_cmp = match (a_score, b_score) {
                (Some(sa), Some(sb)) => sa.cmp(sb),
                _ => Ordering::Equal,
            };

            let base_cmp = if fuzzy_cmp == Ordering::Equal {
                match self.sort_by {
                    SortBy::Name => a.name.cmp(&b.name),
                    SortBy::State => a.active_state.cmp(&b.active_state).then_with(|| a.name.cmp(&b.name)),
                }
            } else {
                fuzzy_cmp
            };

            if self.sort_ascending {
                base_cmp
            } else {
                base_cmp.reverse()
            }
        });

        self.filtered_units = ranked_units.into_iter().map(|(u, _)| u).collect();

        // Rebuild tree items
        self.rebuild_tree_items();

        // Clamp selection
        let total_items = match self.view_mode {
            ViewMode::List => self.filtered_units.len(),
            ViewMode::Tree => self.tree_items.len(),
        };

        if total_items > 0 {
            if self.selected >= total_items {
                self.selected = total_items - 1;
            }
        } else {
            self.selected = 0;
        }
    }

    fn rebuild_tree_items(&mut self,
    ) {
        self.tree_items.clear();

        // Group units by type
        let mut groups: HashMap<String, Vec<UnitInfo>> = HashMap::new();
        for unit in &self.filtered_units {
            let ext = unit.name.split('.').last().unwrap_or("unknown").to_string();
            groups.entry(ext).or_default().push(unit.clone());
        }

        // Sort group names
        let mut group_names: Vec<String> = groups.keys().cloned().collect();
        group_names.sort();

        // On first load, collapse all groups except "service"
        let is_first_load = self.collapsed_groups.is_empty() && !group_names.is_empty();
        if is_first_load {
            for group_name in &group_names {
                if group_name != "service" {
                    self.collapsed_groups.insert(group_name.clone());
                }
            }
        }

        // Build tree items
        for group_name in group_names {
            if let Some(units) = groups.get(&group_name) {
                let active_count = units.iter().filter(|u| u.is_active()).count();

                // Add group header
                self.tree_items.push(TreeItem::Group {
                    name: group_name.clone(),
                    count: units.len(),
                    active: active_count,
                });

                // Add units if group is not collapsed
                if !self.collapsed_groups.contains(&group_name) {
                    for unit in units {
                        self.tree_items.push(TreeItem::Unit { unit: unit.clone() });
                    }
                }
            }
        }
    }

    pub fn selected_unit(&self) -> Option<&UnitInfo> {
        match self.view_mode {
            ViewMode::List => self.filtered_units.get(self.selected),
            ViewMode::Tree => {
                // Find the selected tree item, if it's a unit return it
                if let Some(item) = self.tree_items.get(self.selected) {
                    match item {
                        TreeItem::Unit { unit } => Some(unit),
                        TreeItem::Group { .. } => None,
                    }
                } else {
                    None
                }
            }
        }
    }

    fn toggle_view_mode(&mut self,
    ) {
        self.view_mode = match self.view_mode {
            ViewMode::List => ViewMode::Tree,
            ViewMode::Tree => ViewMode::List,
        };
        self.selected = 0;
        self.scroll_offset = 0;
        if self.view_mode == ViewMode::Tree {
            self.rebuild_tree_items();
        }
    }

    fn toggle_sort(&mut self,
    ) {
        self.sort_by = match self.sort_by {
            SortBy::Name => SortBy::State,
            SortBy::State => SortBy::Name,
        };
        self.apply_filter_and_sort();
    }

    fn toggle_sort_direction(&mut self,
    ) {
        self.sort_ascending = !self.sort_ascending;
        self.apply_filter_and_sort();
    }

    fn toggle_current_group(&mut self,
    ) {
        if self.view_mode != ViewMode::Tree {
            return;
        }

        if let Some(item) = self.tree_items.get(self.selected) {
            if let TreeItem::Group { name, .. } = item {
                let group_name = name.clone();
                if self.collapsed_groups.contains(&group_name) {
                    self.collapsed_groups.remove(&group_name);
                } else {
                    self.collapsed_groups.insert(group_name);
                }
                self.rebuild_tree_items();
            }
        }
    }

    fn expand_all(&mut self,
    ) {
        self.collapsed_groups.clear();
        self.rebuild_tree_items();
    }

    fn collapse_all(&mut self,
    ) {
        // Add all group names to collapsed set
        self.collapsed_groups.clear();
        for item in &self.tree_items {
            if let TreeItem::Group { name, .. } = item {
                self.collapsed_groups.insert(name.clone());
            }
        }
        self.rebuild_tree_items();
    }

    fn move_up(&mut self,
    ) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self,
    ) {
        let max = match self.view_mode {
            ViewMode::List => self.filtered_units.len(),
            ViewMode::Tree => self.tree_items.len(),
        };
        if self.selected + 1 < max {
            self.selected += 1;
        }
    }

    fn go_top(&mut self,
    ) {
        self.selected = 0;
    }

    fn go_bottom(&mut self,
    ) {
        let max = match self.view_mode {
            ViewMode::List => self.filtered_units.len(),
            ViewMode::Tree => self.tree_items.len(),
        };
        if max > 0 {
            self.selected = max - 1;
        }
    }

    fn page_up(&mut self,
        page_size: usize,
    ) {
        self.selected = self.selected.saturating_sub(page_size);
    }

    fn page_down(&mut self,
        page_size: usize,
    ) {
        let max = match self.view_mode {
            ViewMode::List => self.filtered_units.len(),
            ViewMode::Tree => self.tree_items.len(),
        };
        self.selected = (self.selected + page_size).min(max.saturating_sub(1));
    }

    fn get_total_items(&self,
    ) -> usize {
        match self.view_mode {
            ViewMode::List => self.filtered_units.len(),
            ViewMode::Tree => self.tree_items.len(),
        }
    }

    fn move_to_first_leaf_after_filter(&mut self) {
        self.selected = match self.view_mode {
            ViewMode::List => 0,
            ViewMode::Tree => self
                .tree_items
                .iter()
                .position(|item| matches!(item, TreeItem::Unit { .. }))
                .unwrap_or(0),
        };
        self.scroll_offset = 0;
    }

    fn open_detail(&mut self) {
        if let Some(unit) = self.selected_unit().cloned() {
            self.detail_logs = read_recent_unit_logs(&unit.name, 120);
            self.detail_unit = Some(unit);
            self.confirm_action = None;
            self.pending_action = None;
            self.action_status = None;
            self.detail_log_follow = true;
            self.scroll_to_bottom();
        }
    }

    fn close_detail(&mut self) {
        self.detail_unit = None;
        self.confirm_action = None;
        self.pending_action = None;
        self.detail_log_scroll = 0;
        self.detail_log_follow = true;
    }

    fn scroll_to_bottom(&mut self) {
        self.detail_log_scroll = usize::MAX;
    }
}

fn read_recent_unit_logs(unit: &str, max: usize) -> Vec<UnitLogEntry> {
    let mut out = Vec::new();
    unsafe {
        let mut j: *mut c_void = std::ptr::null_mut();
        if sd_journal_open(&mut j as *mut *mut c_void, SD_JOURNAL_LOCAL_ONLY) < 0 || j.is_null() {
            return out;
        }

        let m = format!("_SYSTEMD_UNIT={unit}");
        let _ = sd_journal_add_match(j, m.as_ptr() as *const c_void, m.len());
        let _ = sd_journal_seek_tail(j);

        for _ in 0..max {
            if sd_journal_previous(j) <= 0 {
                break;
            }
            if let Some(entry) = read_journal_entry(j) {
                out.push(entry);
            }
        }
        sd_journal_close(j);
    }
    out.reverse();
    out
}

fn get_journal_field(j: *mut c_void, field: &str) -> Option<String> {
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
    let text = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(data_ptr, len) });
    let prefix = format!("{}=", field);
    text.strip_prefix(&prefix).map(|s| s.to_string())
}

fn read_journal_entry(j: *mut c_void) -> Option<UnitLogEntry> {
    // Get timestamp
    let mut ts_micros: u64 = 0;
    let rc = unsafe { sd_journal_get_realtime_usec(j, &mut ts_micros as *mut u64) };
    if rc < 0 {
        return None;
    }

    let message = get_journal_field(j, "MESSAGE")?;

    // Format timestamp as YYMMDD HH:MM:SS
    let ts_secs = (ts_micros / 1_000_000) as i64;
    let display_time = chrono::DateTime::from_timestamp(ts_secs, 0)
        .map(|dt| {
            let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(dt);
            local.format("%y%m%d %H:%M:%S").to_string()
        })
        .unwrap_or_else(|| "?".to_string());

    Some(UnitLogEntry {
        timestamp_micros: ts_micros,
        display_time,
        message,
    })
}

fn fuzzy_match_score(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }

    // Fast path: contiguous substring match should rank highest.
    if let Some(idx) = haystack.find(needle) {
        return Some(idx);
    }

    // Subsequence fuzzy match: all needle chars must appear in order.
    let mut last_idx = 0usize;
    let mut first_match: Option<usize> = None;
    let mut gap_penalty = 0usize;

    for n in needle.chars() {
        let Some(found_rel) = haystack[last_idx..].find(n) else {
            return None;
        };

        let found_abs = last_idx + found_rel;
        if first_match.is_none() {
            first_match = Some(found_abs);
        }

        gap_penalty += found_rel;
        last_idx = found_abs + n.len_utf8();
    }

    Some(first_match.unwrap_or(0) + gap_penalty * 2 + 100)
}

impl Context for UnitsContext {
    fn name(&self) -> &'static str {
        "Units"
    }

    fn draw(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(4)])
            .split(area);

        // Calculate visible rows
        let visible_rows = chunks[0].height as usize - 3;

        // Unit list
        match self.view_mode {
            ViewMode::List => draw_unit_list(self, f, chunks[0], visible_rows),
            ViewMode::Tree => draw_unit_tree(self, f, chunks[0], visible_rows),
        }

        // Details/status bar
        draw_details(self, f, chunks[1]);

        if self.detail_unit.is_some() {
            draw_unit_popup(self, f, area);
        }
    }

    fn handle_key(&mut self,
        key: KeyEvent,
    ) {
        if self.detail_unit.is_some() {
            if self.confirm_action.is_some() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        self.pending_action = self.confirm_action.take();
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.confirm_action = None;
                    }
                    _ => {}
                }
                return;
            }

            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.close_detail(),
                KeyCode::Char('r') => {
                    if let Some(unit) = &self.detail_unit {
                        self.detail_logs = read_recent_unit_logs(&unit.name, 120);
                        if self.detail_log_follow {
                            self.scroll_to_bottom();
                        }
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.detail_log_scroll = self.detail_log_scroll.saturating_add(1);
                    self.detail_log_follow = false;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.detail_log_scroll = self.detail_log_scroll.saturating_sub(1);
                    self.detail_log_follow = false;
                }
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    self.detail_log_scroll = self.detail_log_scroll.saturating_add(10);
                    self.detail_log_follow = false;
                }
                KeyCode::PageUp | KeyCode::Char('b') => {
                    self.detail_log_scroll = self.detail_log_scroll.saturating_sub(10);
                    self.detail_log_follow = false;
                }
                KeyCode::Char('f') => {
                    self.detail_log_follow = !self.detail_log_follow;
                    if self.detail_log_follow {
                        self.scroll_to_bottom();
                    }
                }
                KeyCode::Char('G') => {
                    self.scroll_to_bottom();
                    self.detail_log_follow = true;
                }
                KeyCode::Char('g') => {
                    self.detail_log_scroll = 0;
                    self.detail_log_follow = false;
                }
                KeyCode::Char('s') => self.confirm_action = Some(UnitAction::Start),
                KeyCode::Char('x') => self.confirm_action = Some(UnitAction::Stop),
                KeyCode::Char('e') => self.confirm_action = Some(UnitAction::Enable),
                KeyCode::Char('d') => self.confirm_action = Some(UnitAction::Disable),
                _ => {}
            }
            return;
        }

        if self.show_filter {
            match key.code {
                KeyCode::Esc => {
                    self.show_filter = false;
                    if let Some(previous) = self.filter_backup.take() {
                        self.filter = previous;
                        self.apply_filter_and_sort();
                    }
                }
                KeyCode::Enter => {
                    self.show_filter = false;
                    self.filter_backup = None;
                    self.move_to_first_leaf_after_filter();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.apply_filter_and_sort();
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.apply_filter_and_sort();
                }
                _ => {}
            }
            return;
        }

        let page_size = 10;

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Char('g') => self.go_top(),
            KeyCode::Char('G') => self.go_bottom(),
            KeyCode::Char(' ') | KeyCode::PageDown => self.page_down(page_size),
            KeyCode::Char('b') | KeyCode::PageUp => self.page_up(page_size),
            KeyCode::Char('/') => {
                if !self.show_filter {
                    self.filter_backup = Some(self.filter.clone());
                }
                self.show_filter = true;
            },
            KeyCode::Char('t') => self.toggle_view_mode(),
            KeyCode::Char('s') => self.toggle_sort(),
            KeyCode::Char('S') => self.toggle_sort_direction(),
            KeyCode::Enter => {
                if self.selected_unit().is_some() {
                    self.open_detail();
                } else {
                    self.toggle_current_group();
                }
            }
            KeyCode::Char('e') => self.expand_all(),
            KeyCode::Char('c') => self.collapse_all(),
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.apply_filter_and_sort();
                }
            }
            _ => {}
        }
    }

    async fn tick(&mut self,
    ) {
        if let Some(action) = self.pending_action.take() {
            if let Some(unit) = self.detail_unit.clone() {
                let result = match action {
                    UnitAction::Start => self.systemd.start_unit(&unit.name).await,
                    UnitAction::Stop => self.systemd.stop_unit(&unit.name).await,
                    UnitAction::Enable => self.systemd.enable_unit(&unit.name).await,
                    UnitAction::Disable => self.systemd.disable_unit(&unit.name).await,
                };

                self.action_status = Some(match result {
                    Ok(_) => format!("{} {}: OK", action.label(), unit.name),
                    Err(e) => format!("{} {}: {}", action.label(), unit.name, e),
                });

                self.refresh(&self.systemd.clone()).await;
                self.detail_logs = read_recent_unit_logs(&unit.name, 120);
                if self.detail_log_follow {
                    self.scroll_to_bottom();
                } else {
                    // Clamp scroll to valid range in case log count changed
                    let visible = 10; // Approximate visible lines
                    let max_scroll = self.detail_logs.len().saturating_sub(visible);
                    self.detail_log_scroll = self.detail_log_scroll.min(max_scroll);
                }
            }
        }
    }
}

fn draw_unit_list(ctx: &UnitsContext, f: &mut Frame, area: Rect, visible_rows: usize) {
    let sort_indicator = match (ctx.sort_by, ctx.sort_ascending) {
        (SortBy::Name, true) => " [name ▲]",
        (SortBy::Name, false) => " [name ▼]",
        (SortBy::State, true) => " [state ▲]",
        (SortBy::State, false) => " [state ▼]",
    };

    let title = if ctx.show_filter {
        format!(" Units [filter: {}]{} ", ctx.filter, sort_indicator)
    } else {
        format!(" Units ({}){} ", ctx.filtered_units.len(), sort_indicator)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL);

    if ctx.loading {
        let loading = Paragraph::new("Loading units...").block(block);
        f.render_widget(loading, area);
        return;
    }

    if let Some(ref error) = ctx.error {
        let error_text = Paragraph::new(format!("Error: {}", error)).block(block);
        f.render_widget(error_text, area);
        return;
    }

    // Calculate scroll offset
    let scroll_offset = if ctx.selected < ctx.scroll_offset {
        ctx.selected
    } else if ctx.selected >= ctx.scroll_offset + visible_rows {
        ctx.selected.saturating_sub(visible_rows - 1)
    } else {
        ctx.scroll_offset
    };

    let header = Row::new(vec!["State", "Name", "Description"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let visible_units: Vec<&UnitInfo> = ctx
        .filtered_units
        .iter()
        .skip(scroll_offset)
        .take(visible_rows)
        .collect();

    let rows: Vec<Row> = visible_units
        .iter()
        .enumerate()
        .map(|(i, unit)| {
            let actual_idx = scroll_offset + i;
            let style = if actual_idx == ctx.selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let state_color = match unit.active_state.as_str() {
                "active" => Color::Green,
                "failed" => Color::Red,
                "inactive" => Color::Gray,
                "activating" => Color::Yellow,
                "deactivating" => Color::Yellow,
                _ => Color::White,
            };

            Row::new(vec![
                Span::styled(unit.state_indicator(), Style::default().fg(state_color)),
                Span::raw(&unit.name),
                Span::styled(&unit.description, Style::default().fg(Color::Gray)),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(rows, vec![
        Constraint::Length(6),
        Constraint::Length(35),
        Constraint::Min(10),
    ])
    .header(header)
    .block(block);

    f.render_widget(table, area);
}

fn draw_unit_tree(ctx: &UnitsContext, f: &mut Frame, area: Rect, visible_rows: usize) {
    let sort_indicator = match (ctx.sort_by, ctx.sort_ascending) {
        (SortBy::Name, true) => " [name ▲]",
        (SortBy::Name, false) => " [name ▼]",
        (SortBy::State, true) => " [state ▲]",
        (SortBy::State, false) => " [state ▼]",
    };

    let expanded_count = ctx.tree_items.len();
    let total_count = ctx.filtered_units.len();
    let group_count = ctx.tree_items.iter().filter(|i| matches!(i, TreeItem::Group { .. })).count();

    let title = if ctx.show_filter {
        format!(" Units [tree] [filter: {}]{} ", ctx.filter, sort_indicator)
    } else {
        format!(" Units [tree] {}/{} in {} groups{} ", expanded_count, total_count, group_count, sort_indicator)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL);

    if ctx.loading {
        let loading = Paragraph::new("Loading units...").block(block);
        f.render_widget(loading, area);
        return;
    }

    if let Some(ref error) = ctx.error {
        let error_text = Paragraph::new(format!("Error: {}", error)).block(block);
        f.render_widget(error_text, area);
        return;
    }

    // Calculate scroll offset
    let scroll_offset = if ctx.selected < ctx.scroll_offset {
        ctx.selected
    } else if ctx.selected >= ctx.scroll_offset + visible_rows {
        ctx.selected.saturating_sub(visible_rows - 1)
    } else {
        ctx.scroll_offset
    };

    let visible_items: Vec<&TreeItem> = ctx
        .tree_items
        .iter()
        .skip(scroll_offset)
        .take(visible_rows)
        .collect();

    let mut text_lines: Vec<Line> = Vec::new();

    for (i, item) in visible_items.iter().enumerate() {
        let actual_idx = scroll_offset + i;
        let is_selected = actual_idx == ctx.selected;
        let style = if is_selected {
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        match item {
            TreeItem::Group { name, count, active } => {
                let is_collapsed = ctx.collapsed_groups.contains(name);
                let icon = if is_collapsed { "▶" } else { "▼" };
                text_lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} {} ({} / {} active)", icon, name, active, count),
                        style.fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            TreeItem::Unit { unit } => {
                let state_color = match unit.active_state.as_str() {
                    "active" => Color::Green,
                    "failed" => Color::Red,
                    "inactive" => Color::Gray,
                    "activating" => Color::Yellow,
                    "deactivating" => Color::Yellow,
                    _ => Color::White,
                };

                text_lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(unit.state_indicator(), Style::default().fg(state_color)),
                    Span::raw(" "),
                    Span::styled(&unit.name, style),
                    Span::raw(" "),
                    Span::styled(&unit.description, Style::default().fg(Color::Gray)),
                ]));
            }
        }
    }

    let text = Paragraph::new(text_lines).block(block);
    f.render_widget(text, area);
}

fn draw_unit_popup(ctx: &UnitsContext, f: &mut Frame, area: Rect) {
    let Some(unit) = ctx.detail_unit.as_ref() else { return; };

    f.render_widget(Clear, area);
    let popup = centered_rect(100, 100, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(6), Constraint::Length(3)])
        .split(popup);

    let meta_lines = vec![
        Line::from(format!("Name: {}", unit.name)),
        Line::from(format!("Description: {}", unit.description)),
        Line::from(format!("Load: {}", unit.load_state)),
        Line::from(format!("Active: {}", unit.active_state)),
        Line::from(format!("Sub: {}", unit.sub_state)),
        Line::from("Actions: s=start x=stop e=enable d=disable r=refresh f=follow g=top G=bottom q=back"),
    ];

    f.render_widget(
        Paragraph::new(meta_lines)
            .block(Block::default().title(" Unit Metadata ").borders(Borders::ALL)),
        chunks[0],
    );

    let log_lines: Vec<Line> = if ctx.detail_logs.is_empty() {
        vec![Line::from("No logs for this unit")]
    } else {
        ctx.detail_logs.iter().map(|entry| {
            Line::from(vec![
                Span::styled(
                    format!("{:15} ", entry.display_time),
                    Style::default().fg(Color::Gray),
                ),
                Span::raw(&entry.message),
            ])
        }).collect()
    };

    let visible = chunks[1].height.saturating_sub(2) as usize;
    let max_scroll = log_lines.len().saturating_sub(visible);
    let scroll = ctx.detail_log_scroll.min(max_scroll) as u16;

    f.render_widget(
        Paragraph::new(log_lines)
            .scroll((scroll, 0))
            .block(
                Block::default()
                    .title(format!(
                        " Recent Logs [{} / {}] {}{} ",
                        scroll,
                        max_scroll,
                        if ctx.detail_log_follow { "[follow] " } else { "" },
                        if ctx.detail_log_scroll > max_scroll { "[bottom]" } else { "" }
                    ))
                    .borders(Borders::ALL),
            ),
        chunks[1],
    );

    let status = if let Some(confirm) = ctx.confirm_action {
        format!("Confirm {} on {} ? [y/n]", confirm.label(), unit.name)
    } else {
        ctx.action_status
            .clone()
            .unwrap_or_else(|| "Ready".to_string())
    };

    f.render_widget(
        Paragraph::new(status)
            .block(Block::default().title(" Status ").borders(Borders::ALL)),
        chunks[2],
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn draw_details(ctx: &UnitsContext, f: &mut Frame, area: Rect) {
    let mode_str = match ctx.view_mode {
        ViewMode::List => "[list]",
        ViewMode::Tree => "[tree]",
    };

    let block = Block::default()
        .title(format!(" Details {} ", mode_str))
        .borders(Borders::ALL);

    if let Some(unit) = ctx.selected_unit() {
        let state_color = match unit.active_state.as_str() {
            "active" => Color::Green,
            "failed" => Color::Red,
            _ => Color::Gray,
        };

        let lines = vec![
            Line::from(vec![
                Span::raw("Name: "),
                Span::styled(&unit.name, Style::default().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::raw("State: "),
                Span::styled(
                    format!("{} ({}/{})", unit.state_indicator(), unit.active_state, unit.sub_state),
                    Style::default().fg(state_color),
                ),
            ]),
            Line::from(vec![Span::raw(format!("Load: {}", unit.load_state))]),
            Line::from(vec![
                Span::raw("Enter:toggle e:expand-all c:collapse-all t:view s:sort"),
            ]),
        ];

        let details = Paragraph::new(lines).block(block);
        f.render_widget(details, area);
    } else {
        // Check if we're on a group
        let group_name = if ctx.view_mode == ViewMode::Tree {
            ctx.tree_items.get(ctx.selected).and_then(|item| {
                match item {
                    TreeItem::Group { name, .. } => Some(name.clone()),
                    _ => None,
                }
            })
        } else {
            None
        };

        let lines = if let Some(name) = group_name {
            vec![
                Line::from(vec![
                    Span::raw("Group: "),
                    Span::styled(name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                ]),
                Line::from("Press Enter to toggle expansion"),
                Line::from("e:expand-all c:collapse-all t:view s:sort"),
            ]
        } else {
            vec![
                Line::from("No unit selected"),
                Line::from("e:expand-all c:collapse-all t:view s:sort"),
            ]
        };
        let empty = Paragraph::new(lines).block(block);
        f.render_widget(empty, area);
    }
}
