use crate::contexts::Context;
use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::ptr;

pub struct NetworkInfo {
    interfaces: Vec<Interface>,
    routes: Vec<Route>,
}

#[derive(Clone)]
pub struct Interface {
    name: String,
    state: String,
    mac: Option<String>,
    mtu: Option<u32>,
    ipv4: Vec<String>,
    ipv6: Vec<String>,
    rx_bytes: u64,
    tx_bytes: u64,
}

#[derive(Clone)]
pub struct Route {
    destination: String,
    gateway: Option<String>,
    interface: String,
    metric: Option<u32>,
}

impl NetworkInfo {
    fn gather() -> Result<Self> {
        let interfaces = Self::get_interfaces()?;
        let routes = Self::get_routes()?;

        Ok(Self {
            interfaces,
            routes,
        })
    }

    fn get_interfaces() -> Result<Vec<Interface>> {
        let mut interfaces = Vec::new();
        let addr_map = Self::get_ip_addresses()?;

        if let Ok(dir) = fs::read_dir("/sys/class/net") {
            for entry in dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "lo" {
                    continue;
                }

                let iface_path = entry.path();
                let state = fs::read_to_string(iface_path.join("operstate"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| "unknown".to_string());

                let mac = fs::read_to_string(iface_path.join("address"))
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty() && s != "00:00:00:00:00:00");

                let mtu = fs::read_to_string(iface_path.join("mtu"))
                    .ok()
                    .and_then(|s| s.trim().parse().ok());

                let rx_bytes = Self::read_stat(&iface_path, "statistics/rx_bytes");
                let tx_bytes = Self::read_stat(&iface_path, "statistics/tx_bytes");

                let (ipv4, ipv6) = addr_map.get(&name).cloned().unwrap_or_default();

                interfaces.push(Interface {
                    name,
                    state,
                    mac,
                    mtu,
                    ipv4,
                    ipv6,
                    rx_bytes,
                    tx_bytes,
                });
            }
        }

        interfaces.sort_by(|a, b| {
            let a_up = a.state == "up";
            let b_up = b.state == "up";
            b_up.cmp(&a_up).then_with(|| a.name.cmp(&b.name))
        });

        Ok(interfaces)
    }

    fn read_stat(path: &std::path::Path, file: &str) -> u64 {
        fs::read_to_string(path.join(file))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    fn get_ip_addresses() -> Result<HashMap<String, (Vec<String>, Vec<String>)>> {
        let mut map: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();

        let mut ifap: *mut libc::ifaddrs = ptr::null_mut();
        let rc = unsafe { libc::getifaddrs(&mut ifap as *mut *mut libc::ifaddrs) };
        if rc != 0 {
            return Ok(map);
        }

        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = unsafe { &*cur };

            if !ifa.ifa_name.is_null() && !ifa.ifa_addr.is_null() {
                let name = unsafe { CStr::from_ptr(ifa.ifa_name) }
                    .to_string_lossy()
                    .to_string();

                if name != "lo" {
                    let family = unsafe { (*ifa.ifa_addr).sa_family as i32 };
                    let entry = map.entry(name).or_insert_with(|| (Vec::new(), Vec::new()));

                    if family == libc::AF_INET {
                        let sa = unsafe { *(ifa.ifa_addr as *const libc::sockaddr_in) };
                        let ip = Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr)).to_string();
                        if !entry.0.contains(&ip) {
                            entry.0.push(ip);
                        }
                    } else if family == libc::AF_INET6 {
                        let sa6 = unsafe { *(ifa.ifa_addr as *const libc::sockaddr_in6) };
                        let ip = Ipv6Addr::from(sa6.sin6_addr.s6_addr).to_string();
                        if !ip.starts_with("fe80:") && !entry.1.contains(&ip) {
                            entry.1.push(ip);
                        }
                    }
                }
            }

            cur = unsafe { (*cur).ifa_next };
        }

        unsafe { libc::freeifaddrs(ifap) };
        Ok(map)
    }

    fn get_routes() -> Result<Vec<Route>> {
        let mut routes = Vec::new();

        if let Ok(content) = fs::read_to_string("/proc/net/route") {
            for line in content.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 8 {
                    let iface = parts[0].to_string();
                    let dest = parts[1];
                    let gateway = parts[2];

                    let dest_ip = Self::hex_to_ip(dest);
                    let gateway_ip = if gateway != "00000000" {
                        Some(Self::hex_to_ip(gateway))
                    } else {
                        None
                    };

                    let metric = parts[6].parse().ok();

                    routes.push(Route {
                        destination: if dest_ip == "0.0.0.0" {
                            "default".to_string()
                        } else {
                            dest_ip
                        },
                        gateway: gateway_ip,
                        interface: iface,
                        metric,
                    });
                }
            }
        }

        Ok(routes)
    }

    fn extract_json_string(content: &str, key: &str) -> Option<String> {
        if let Some(start) = content.find(key) {
            let after_key = &content[start + key.len()..];
            if let Some(end) = after_key.find("\"") {
                return Some(after_key[..end].to_string());
            }
        }
        None
    }

    fn extract_json_u32(content: &str, key: &str) -> Option<u32> {
        if let Some(start) = content.find(key) {
            let after_key = &content[start + key.len()..];
            let end = after_key.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_key.len());
            return after_key[..end].parse().ok();
        }
        None
    }

    fn hex_to_ip(hex: &str) -> String {
        if hex.len() != 8 {
            return "invalid".to_string();
        }

        let octets: Vec<u8> = (0..8)
            .step_by(2)
            .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
            .collect();

        if octets.len() == 4 {
            format!("{}.{}.{}.{}", octets[3], octets[2], octets[1], octets[0])
        } else {
            "invalid".to_string()
        }
    }

    fn format_bytes(bytes: u64) -> String {
        const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
        let mut size = bytes as f64;
        let mut unit_idx = 0;

        while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
            size /= 1024.0;
            unit_idx += 1;
        }

        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}

pub struct NetworkContext {
    info: Option<NetworkInfo>,
    error: Option<String>,
    selected_interface: usize,
    scroll_offset: usize,
}

impl NetworkContext {
    pub fn new() -> Self {
        let (info, error) = match NetworkInfo::gather() {
            Ok(info) => (Some(info), None),
            Err(e) => (None, Some(format!("Failed to gather network info: {}", e))),
        };

        Self {
            info,
            error,
            selected_interface: 0,
            scroll_offset: 0,
        }
    }

    fn refresh(&mut self,
    ) {
        let (info, error) = match NetworkInfo::gather() {
            Ok(info) => (Some(info), None),
            Err(e) => (None, Some(format!("Failed to gather network info: {}", e))),
        };
        self.info = info;
        self.error = error;
        self.selected_interface = 0;
        self.scroll_offset = 0;
    }

    fn move_up(&mut self,
    ) {
        if self.selected_interface > 0 {
            self.selected_interface -= 1;
        }
    }

    fn move_down(&mut self,
    ) {
        if let Some(ref info) = self.info {
            if !info.interfaces.is_empty() && self.selected_interface + 1 < info.interfaces.len() {
                self.selected_interface += 1;
            }
        }
    }

    fn page_up(&mut self,
    ) {
        self.selected_interface = self.selected_interface.saturating_sub(5);
    }

    fn page_down(&mut self,
    ) {
        if let Some(ref info) = self.info {
            if !info.interfaces.is_empty() {
                self.selected_interface = (self.selected_interface + 5).min(info.interfaces.len() - 1);
            }
        }
    }

    fn go_top(&mut self,
    ) {
        self.selected_interface = 0;
    }

    fn go_bottom(&mut self,
    ) {
        if let Some(ref info) = self.info {
            if !info.interfaces.is_empty() {
                self.selected_interface = info.interfaces.len() - 1;
            }
        }
    }
}

impl Context for NetworkContext {
    fn name(&self) -> &'static str {
        "Network"
    }

    fn draw(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(6)])
            .split(area);

        // Interface list
        draw_interfaces(self, f, chunks[0]);

        // Routes
        draw_routes(self, f, chunks[1]);
    }

    fn handle_key(&mut self, key: KeyEvent,
    ) {
        match key.code {
            crossterm::event::KeyCode::Char('r') => self.refresh(),
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => self.move_down(),
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => self.move_up(),
            crossterm::event::KeyCode::Char(' ') | crossterm::event::KeyCode::PageDown => self.page_down(),
            crossterm::event::KeyCode::Char('b') | crossterm::event::KeyCode::PageUp => self.page_up(),
            crossterm::event::KeyCode::Char('g') => self.go_top(),
            crossterm::event::KeyCode::Char('G') => self.go_bottom(),
            _ => {}
        }
    }

    async fn tick(&mut self) {}
}

fn draw_interfaces(ctx: &NetworkContext, f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Network Interfaces ")
        .borders(Borders::ALL);

    if let Some(ref error) = ctx.error {
        let error_text = Paragraph::new(format!("Error: {}", error)).block(block);
        f.render_widget(error_text, area);
        return;
    }

    if let Some(ref info) = ctx.info {
        if info.interfaces.is_empty() {
            let empty = Paragraph::new("No network interfaces found").block(block);
            f.render_widget(empty, area);
            return;
        }

        // Build text lines for multiline display
        let mut lines: Vec<Line> = Vec::new();

        for (i, iface) in info.interfaces.iter().enumerate() {
            let is_selected = i == ctx.selected_interface;

            let state_color = match iface.state.as_str() {
                "up" => Color::Green,
                "down" => Color::Red,
                _ => Color::Yellow,
            };

            let name_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            };

            // Interface header line with stats
            let header_line = Line::from(vec![
                Span::styled(format!("{:12} ", iface.name), name_style),
                Span::styled(
                    format!("[{:8}] ", iface.state),
                    Style::default().fg(state_color),
                ),
                Span::styled(
                    format!("RX: {:>10}  ", NetworkInfo::format_bytes(iface.rx_bytes)),
                    Style::default().fg(Color::Blue),
                ),
                Span::styled(
                    format!("TX: {:>10}", NetworkInfo::format_bytes(iface.tx_bytes)),
                    Style::default().fg(Color::Green),
                ),
            ]);
            lines.push(header_line);

            // MAC address line (if available)
            if let Some(ref mac) = iface.mac {
                lines.push(Line::from(vec![
                    Span::raw("             MAC: "),
                    Span::styled(mac, Style::default().fg(Color::Gray)),
                ]));
            }

            // IPv4 addresses
            for (j, ip) in iface.ipv4.iter().enumerate() {
                let label = if j == 0 { "IPv4: " } else { "      " };
                lines.push(Line::from(vec![
                    Span::raw(format!("             {}{}", label, ip)),
                ]));
            }

            // IPv6 addresses (with enough width)
            for (j, ip) in iface.ipv6.iter().enumerate() {
                let label = if j == 0 { "IPv6: " } else { "      " };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("             {}{}", label, ip),
                        Style::default().fg(Color::Yellow),
                    ),
                ]));
            }

            // Empty line between interfaces (except last)
            if i < info.interfaces.len() - 1 {
                lines.push(Line::from(""));
            }
        }

        let text = Paragraph::new(lines).block(block);
        f.render_widget(text, area);
    } else {
        let loading = Paragraph::new("Loading...").block(block);
        f.render_widget(loading, area);
    }
}

fn draw_routes(ctx: &NetworkContext, f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Routing Table ")
        .borders(Borders::ALL);

    if let Some(ref info) = ctx.info {
        if info.routes.is_empty() {
            let empty = Paragraph::new("No routes found").block(block);
            f.render_widget(empty, area);
            return;
        }

        // Show only default and a few routes to fit in the space
        let important_routes: Vec<&Route> = info
            .routes
            .iter()
            .filter(|r| r.destination == "default")
            .chain(info.routes.iter().filter(|r| r.destination != "default").take(3))
            .collect();

        let mut lines: Vec<Line> = Vec::new();

        for route in important_routes {
            let dest = if route.destination == "default" {
                Span::styled("default", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            } else {
                Span::raw(route.destination.clone())
            };

            let gateway = route.gateway.clone().unwrap_or_else(|| "-".to_string());
            let metric = route.metric.map(|m| format!("{}", m)).unwrap_or_else(|| "-".to_string());

            lines.push(Line::from(vec![
                dest,
                Span::raw(format!(" via {} on {} (metric {})", gateway, route.interface, metric)),
            ]));
        }

        let text = Paragraph::new(lines).block(block);
        f.render_widget(text, area);
    } else {
        let loading = Paragraph::new("Loading...").block(block);
        f.render_widget(loading, area);
    }
}
