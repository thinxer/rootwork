use crate::contexts::Context;
use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph, Row, Table},
};
use std::path::Path;

pub struct BootInfo {
    systemd_boot: bool,
    firmware: String,
    loader_version: String,
    secure_boot: String,
    setup_mode: String,
    entries: Vec<BootEntry>,
}

pub struct BootEntry {
    id: String,
    title: String,
    version: Option<String>,
    machine_id: Option<String>,
    is_default: bool,
}

impl BootInfo {
    fn gather() -> Result<Self> {
        Self::from_fallback()
    }

    fn from_fallback() -> Result<Self> {
        // Fallback: check /boot or /efi for entries
        let entries = Self::scan_boot_entries()?;

        // Check for secure boot via efivars if available
        let secure_boot = Self::check_secure_boot();

        Ok(Self {
            systemd_boot: Path::new("/boot/EFI/systemd").exists()
                || Path::new("/efi/EFI/systemd").exists(),
            firmware: "unknown".to_string(),
            loader_version: "unknown".to_string(),
            secure_boot,
            setup_mode: "unknown".to_string(),
            entries,
        })
    }

    fn scan_boot_entries() -> Result<Vec<BootEntry>> {
        let mut entries = Vec::new();

        // Check common bootloader paths
        let paths = ["/boot/loader/entries", "/efi/loader/entries"];

        for path in &paths {
            if let Ok(dir) = std::fs::read_dir(path) {
                for entry in dir.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "conf").unwrap_or(false) {
                        if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                            entries.push(BootEntry {
                                id: name.to_string(),
                                title: name.to_string(),
                                version: None,
                                machine_id: None,
                                is_default: false,
                            });
                        }
                    }
                }
            }
        }

        Ok(entries)
    }

    fn check_secure_boot() -> String {
        // Check /sys/firmware/efi/efivars/SecureBoot-*
        match std::fs::read_dir("/sys/firmware/efi/efivars") {
            Ok(dir) => {
                for entry in dir.flatten() {
                    let name = entry.file_name();
                    if let Some(name_str) = name.to_str() {
                        if name_str.starts_with("SecureBoot-") {
                            // Read the value - format is attribute (4 bytes) + data
                            if let Ok(data) = std::fs::read(entry.path()) {
                                if data.len() >= 5 {
                                    // Last byte is the value
                                    return match data[data.len() - 1] {
                                        1 => "enabled".to_string(),
                                        0 => "disabled".to_string(),
                                        _ => "unknown".to_string(),
                                    };
                                }
                            }
                        }
                    }
                }
                "unknown".to_string()
            }
            Err(_) => "not available".to_string(),
        }
    }
}

pub struct BootContext {
    info: Option<BootInfo>,
    error: Option<String>,
    selected_entry: usize,
}

impl BootContext {
    pub fn new() -> Self {
        let (info, error) = match BootInfo::gather() {
            Ok(info) => (Some(info), None),
            Err(e) => (None, Some(format!("Failed to gather boot info: {}", e))),
        };

        Self {
            info,
            error,
            selected_entry: 0,
        }
    }

    fn refresh(&mut self) {
        let (info, error) = match BootInfo::gather() {
            Ok(info) => (Some(info), None),
            Err(e) => (None, Some(format!("Failed to gather boot info: {}", e))),
        };
        self.info = info;
        self.error = error;
        self.selected_entry = 0;
    }

    fn move_up(&mut self) {
        if let Some(ref info) = self.info {
            if !info.entries.is_empty() && self.selected_entry > 0 {
                self.selected_entry -= 1;
            }
        }
    }

    fn move_down(&mut self) {
        if let Some(ref info) = self.info {
            if !info.entries.is_empty() && self.selected_entry + 1 < info.entries.len() {
                self.selected_entry += 1;
            }
        }
    }
}

impl Context for BootContext {
    fn name(&self) -> &'static str {
        "Boot"
    }

    fn draw(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Min(0)])
            .split(area);

        // Boot firmware info
        draw_firmware_info(self, f, chunks[0]);

        // Boot entries
        draw_boot_entries(self, f, chunks[1]);
    }

    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            crossterm::event::KeyCode::Char('r') => self.refresh(),
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                self.move_down()
            }
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => self.move_up(),
            _ => {}
        }
    }

    async fn tick(&mut self) {}
}

fn draw_firmware_info(ctx: &BootContext, f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Firmware / Bootloader ")
        .borders(Borders::ALL);

    if let Some(ref error) = ctx.error {
        let error_text = Paragraph::new(format!("Error: {}", error)).block(block);
        f.render_widget(error_text, area);
        return;
    }

    if let Some(ref info) = ctx.info {
        let bootloader_status = if info.systemd_boot {
            format!("systemd-boot ({}) ✓", info.loader_version)
        } else {
            "other".to_string()
        };

        let rows = vec![
            Row::new(vec!["Firmware", &info.firmware]),
            Row::new(vec!["Bootloader", &bootloader_status]),
            Row::new(vec!["Secure Boot", &info.secure_boot]),
            Row::new(vec!["Setup Mode", &info.setup_mode]),
        ];

        let table =
            Table::new(rows, vec![Constraint::Length(14), Constraint::Min(40)]).block(block);

        f.render_widget(table, area);
    } else {
        let loading = Paragraph::new("Loading...").block(block);
        f.render_widget(loading, area);
    }
}

fn draw_boot_entries(ctx: &BootContext, f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Boot Entries ")
        .borders(Borders::ALL);

    if let Some(ref info) = ctx.info {
        if info.entries.is_empty() {
            let empty = Paragraph::new("No boot entries found").block(block);
            f.render_widget(empty, area);
            return;
        }

        let header = Row::new(vec!["Default", "Title", "Version", "ID"])
            .style(Style::default().add_modifier(Modifier::BOLD));

        let rows: Vec<Row> = info
            .entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let style = if i == ctx.selected_entry {
                    Style::default()
                        .bg(crate::palette::dark_gray())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let default_indicator = if entry.is_default {
                    Span::styled("★", Style::default().fg(crate::palette::yellow()))
                } else {
                    Span::raw("")
                };

                Row::new(vec![
                    default_indicator,
                    Span::raw(entry.title.clone()),
                    Span::raw(entry.version.clone().unwrap_or_else(|| "-".to_string())),
                    Span::styled(
                        entry.id.clone(),
                        Style::default().fg(crate::palette::gray()),
                    ),
                ])
                .style(style)
            })
            .collect();

        let table = Table::new(
            rows,
            vec![
                Constraint::Length(8),
                Constraint::Length(30),
                Constraint::Length(15),
                Constraint::Min(20),
            ],
        )
        .header(header)
        .block(block);

        f.render_widget(table, area);
    } else {
        let loading = Paragraph::new("Loading...").block(block);
        f.render_widget(loading, area);
    }
}
