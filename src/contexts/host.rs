use crate::contexts::Context;
use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph, Row, Table},
};
use std::fs;
use zbus::blocking::{Connection, Proxy};

pub struct HostInfo {
    hostname: String,
    static_hostname: String,
    timezone: String,
    locale: String,
    os_name: String,
    os_version: String,
    uptime: String,
    ntp_enabled: String,
    ntp_sync: String,
}

impl HostInfo {
    fn gather() -> anyhow::Result<Self> {
        let conn = Connection::system()?;

        // hostname1
        let hostname = dbus_get_string(
            &conn,
            "org.freedesktop.hostname1",
            "/org/freedesktop/hostname1",
            "org.freedesktop.hostname1",
            "Hostname",
        )
        .unwrap_or_else(|| "unknown".to_string());

        let static_hostname = dbus_get_string(
            &conn,
            "org.freedesktop.hostname1",
            "/org/freedesktop/hostname1",
            "org.freedesktop.hostname1",
            "StaticHostname",
        )
        .unwrap_or_else(|| hostname.clone());

        // timedate1
        let timezone = dbus_get_string(
            &conn,
            "org.freedesktop.timedate1",
            "/org/freedesktop/timedate1",
            "org.freedesktop.timedate1",
            "Timezone",
        )
        .unwrap_or_else(|| "unknown".to_string());

        let ntp_enabled = dbus_get_bool(
            &conn,
            "org.freedesktop.timedate1",
            "/org/freedesktop/timedate1",
            "org.freedesktop.timedate1",
            "NTP",
        )
        .map(|v| if v { "enabled" } else { "disabled" }.to_string())
        .unwrap_or_else(|| "unknown".to_string());

        let ntp_sync = dbus_get_bool(
            &conn,
            "org.freedesktop.timedate1",
            "/org/freedesktop/timedate1",
            "org.freedesktop.timedate1",
            "NTPSynchronized",
        )
        .map(|v| if v { "yes" } else { "no" }.to_string())
        .unwrap_or_else(|| "unknown".to_string());

        // locale1
        let locale = dbus_get_locale(&conn).unwrap_or_else(|| "unknown".to_string());

        let (os_name, os_version) = Self::get_os_info();
        let uptime = Self::get_uptime();

        Ok(Self {
            hostname,
            static_hostname,
            timezone,
            locale,
            os_name,
            os_version,
            uptime,
            ntp_enabled,
            ntp_sync,
        })
    }

    fn get_os_info() -> (String, String) {
        if let Ok(content) = fs::read_to_string("/etc/os-release") {
            let mut name = "unknown".to_string();
            let mut version = "unknown".to_string();

            for line in content.lines() {
                if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
                    name = v.trim_matches('"').to_string();
                } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
                    version = v.trim_matches('"').to_string();
                }
            }

            (name, version)
        } else {
            ("unknown".to_string(), "unknown".to_string())
        }
    }

    fn get_uptime() -> String {
        if let Ok(content) = fs::read_to_string("/proc/uptime") {
            let seconds: f64 = content
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);

            let days = (seconds / 86400.0) as u64;
            let hours = ((seconds % 86400.0) / 3600.0) as u64;
            let minutes = ((seconds % 3600.0) / 60.0) as u64;

            if days > 0 {
                format!("{}d {}h {}m", days, hours, minutes)
            } else if hours > 0 {
                format!("{}h {}m", hours, minutes)
            } else {
                format!("{}m", minutes)
            }
        } else {
            "unknown".to_string()
        }
    }
}

fn dbus_get_string(
    conn: &Connection,
    service: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> Option<String> {
    let proxy = Proxy::new(conn, service, path, interface).ok()?;
    proxy.get_property::<String>(property).ok()
}

fn dbus_get_bool(
    conn: &Connection,
    service: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> Option<bool> {
    let proxy = Proxy::new(conn, service, path, interface).ok()?;
    proxy.get_property::<bool>(property).ok()
}

fn dbus_get_locale(conn: &Connection) -> Option<String> {
    let proxy = Proxy::new(
        conn,
        "org.freedesktop.locale1",
        "/org/freedesktop/locale1",
        "org.freedesktop.locale1",
    )
    .ok()?;

    let values = proxy.get_property::<Vec<String>>("Locale").ok()?;
    values
        .iter()
        .find(|s| s.starts_with("LANG="))
        .map(|s| s.trim_start_matches("LANG=").to_string())
        .or_else(|| values.first().cloned())
}

pub struct HostContext {
    info: Option<HostInfo>,
    error: Option<String>,
}

impl HostContext {
    pub fn new() -> Self {
        let (info, error) = match HostInfo::gather() {
            Ok(info) => (Some(info), None),
            Err(e) => (None, Some(format!("Failed to gather host info: {}", e))),
        };

        Self { info, error }
    }

    fn refresh(&mut self) {
        let (info, error) = match HostInfo::gather() {
            Ok(info) => (Some(info), None),
            Err(e) => (None, Some(format!("Failed to gather host info: {}", e))),
        };
        self.info = info;
        self.error = error;
    }
}

impl Context for HostContext {
    fn name(&self) -> &'static str {
        "Host"
    }

    fn draw(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" Host Information ")
            .borders(Borders::ALL);

        if let Some(ref error) = self.error {
            let error_text = Paragraph::new(format!("Error: {}", error)).block(block);
            f.render_widget(error_text, area);
            return;
        }

        if let Some(ref info) = self.info {
            let os_str = format!("{} {}", info.os_name, info.os_version);

            let rows = vec![
                Row::new(vec!["Hostname", &info.hostname]),
                Row::new(vec!["Static Hostname", &info.static_hostname]),
                Row::new(vec!["Operating System", &os_str]),
                Row::new(vec!["Timezone", &info.timezone]),
                Row::new(vec!["Locale", &info.locale]),
                Row::new(vec!["Uptime", &info.uptime]),
                Row::new(vec!["NTP Enabled", &info.ntp_enabled]),
                Row::new(vec!["NTP Synchronized", &info.ntp_sync]),
            ];

            let table = Table::new(rows, vec![Constraint::Length(20), Constraint::Min(30)])
                .header(
                    Row::new(vec!["Property", "Value"])
                        .style(Style::default().add_modifier(Modifier::BOLD)),
                )
                .block(block)
                .row_highlight_style(Style::default().bg(crate::palette::dark_gray()));

            f.render_widget(table, area);
        } else {
            let loading = Paragraph::new("Loading...").block(block);
            f.render_widget(loading, area);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if let crossterm::event::KeyCode::Char('r') = key.code {
            self.refresh();
        }
    }

    async fn tick(&mut self) {}
}
