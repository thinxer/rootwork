use crate::contexts::Context;
use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use zbus::blocking::{Connection, Proxy};

pub struct DnsInfo {
    current_dns: Vec<String>,
    fallback_dns: Vec<String>,
    dnssec: String,
    dnsovertls: String,
    search_domains: Vec<String>,
    interface_dns: Vec<InterfaceDns>,
}

#[derive(Clone)]
pub struct InterfaceDns {
    name: String,
    dns_servers: Vec<String>,
    search_domains: Vec<String>,
}

impl DnsInfo {
    fn gather() -> Result<Self> {
        Self::from_resolved_dbus().or_else(|_| Self::from_resolv_conf())
    }

    fn from_resolved_dbus() -> Result<Self> {
        let conn = Connection::system()?;
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.resolve1",
            "/org/freedesktop/resolve1",
            "org.freedesktop.resolve1.Manager",
        )?;

        let dns: Vec<(i32, i32, Vec<u8>)> = proxy.get_property("DNS")?;
        let fallback_dns_raw: Vec<(i32, i32, Vec<u8>)> = proxy.get_property("FallbackDNS")?;
        let domains: Vec<(i32, String, bool)> = proxy.get_property("Domains")?;
        let dnssec: String = proxy.get_property("DNSSEC").unwrap_or_else(|_| "unknown".to_string());
        let dnsovertls: String = proxy
            .get_property("DNSOverTLS")
            .unwrap_or_else(|_| "unknown".to_string());

        let mut global_dns = BTreeSet::new();
        let mut if_servers: BTreeMap<i32, BTreeSet<String>> = BTreeMap::new();

        for (ifindex, family, bytes) in dns {
            if let Some(ip) = decode_ip(family, &bytes) {
                if ifindex == 0 {
                    global_dns.insert(ip.clone());
                } else {
                    if_servers.entry(ifindex).or_default().insert(ip);
                }
            }
        }

        let mut fallback_dns = BTreeSet::new();
        for (_ifindex, family, bytes) in fallback_dns_raw {
            if let Some(ip) = decode_ip(family, &bytes) {
                fallback_dns.insert(ip);
            }
        }

        let mut global_domains = BTreeSet::new();
        let mut if_domains: BTreeMap<i32, BTreeSet<String>> = BTreeMap::new();
        for (ifindex, domain, _route_only) in domains {
            if domain.is_empty() {
                continue;
            }
            if ifindex == 0 {
                global_domains.insert(domain);
            } else {
                if_domains.entry(ifindex).or_default().insert(domain);
            }
        }

        let mut interfaces = BTreeSet::new();
        interfaces.extend(if_servers.keys().copied());
        interfaces.extend(if_domains.keys().copied());

        let interface_dns = interfaces
            .into_iter()
            .map(|ifindex| InterfaceDns {
                name: ifindex_to_name(ifindex).unwrap_or_else(|| format!("if#{ifindex}")),
                dns_servers: if_servers
                    .remove(&ifindex)
                    .map(|s| s.into_iter().collect())
                    .unwrap_or_default(),
                search_domains: if_domains
                    .remove(&ifindex)
                    .map(|s| s.into_iter().collect())
                    .unwrap_or_default(),
            })
            .collect();

        Ok(Self {
            current_dns: global_dns.into_iter().collect(),
            fallback_dns: fallback_dns.into_iter().collect(),
            dnssec,
            dnsovertls,
            search_domains: global_domains.into_iter().collect(),
            interface_dns,
        })
    }

    fn from_resolv_conf() -> Result<Self> {
        let content = std::fs::read_to_string("/etc/resolv.conf")?;
        let mut current_dns = Vec::new();
        let mut search_domains = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("nameserver") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() > 1 {
                    current_dns.push(parts[1].to_string());
                }
            } else if trimmed.starts_with("search") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                search_domains.extend(parts[1..].iter().map(|s| s.to_string()));
            }
        }

        Ok(Self {
            current_dns,
            fallback_dns: Vec::new(),
            dnssec: "unknown".to_string(),
            dnsovertls: "unknown".to_string(),
            search_domains,
            interface_dns: Vec::new(),
        })
    }
}

fn decode_ip(family: i32, bytes: &[u8]) -> Option<String> {
    match family {
        libc::AF_INET => {
            if bytes.len() == 4 {
                Some(IpAddr::V4(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])).to_string())
            } else {
                None
            }
        }
        libc::AF_INET6 => {
            if bytes.len() == 16 {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(bytes);
                Some(IpAddr::V6(Ipv6Addr::from(octets)).to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn ifindex_to_name(ifindex: i32) -> Option<String> {
    if ifindex <= 0 {
        return None;
    }

    let mut buf = [0i8; libc::IF_NAMESIZE];
    let ptr = unsafe { libc::if_indextoname(ifindex as u32, buf.as_mut_ptr()) };
    if ptr.is_null() {
        return None;
    }

    let cstr = unsafe { std::ffi::CStr::from_ptr(ptr) };
    cstr.to_str().ok().map(|s| s.to_string())
}

pub struct DnsContext {
    info: Option<DnsInfo>,
    error: Option<String>,
    selected_interface: usize,
}

impl DnsContext {
    pub fn new() -> Self {
        let (info, error) = match DnsInfo::gather() {
            Ok(info) => (Some(info), None),
            Err(e) => (None, Some(format!("Failed to gather DNS info: {}", e))),
        };

        Self {
            info,
            error,
            selected_interface: 0,
        }
    }

    fn refresh(&mut self) {
        let (info, error) = match DnsInfo::gather() {
            Ok(info) => (Some(info), None),
            Err(e) => (None, Some(format!("Failed to gather DNS info: {}", e))),
        };
        self.info = info;
        self.error = error;
        self.selected_interface = 0;
    }

    fn move_up(&mut self) {
        if let Some(ref info) = self.info
            && !info.interface_dns.is_empty()
            && self.selected_interface > 0
        {
            self.selected_interface -= 1;
        }
    }

    fn move_down(&mut self) {
        if let Some(ref info) = self.info
            && !info.interface_dns.is_empty()
            && self.selected_interface + 1 < info.interface_dns.len()
        {
            self.selected_interface += 1;
        }
    }
}

impl Context for DnsContext {
    fn name(&self) -> &'static str {
        "DNS"
    }

    fn draw(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(0)])
            .split(area);

        draw_global_dns(self, f, chunks[0]);
        draw_interface_dns(self, f, chunks[1]);
    }

    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            crossterm::event::KeyCode::Char('r') => self.refresh(),
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => self.move_down(),
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => self.move_up(),
            _ => {}
        }
    }

    async fn tick(&mut self) {}
}

fn draw_global_dns(ctx: &DnsContext, f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Global DNS Settings ")
        .borders(Borders::ALL);

    if let Some(ref error) = ctx.error {
        let error_text = Paragraph::new(format!("Error: {}", error)).block(block);
        f.render_widget(error_text, area);
        return;
    }

    if let Some(ref info) = ctx.info {
        let dns_str = if info.current_dns.is_empty() {
            "None configured".to_string()
        } else {
            info.current_dns.join(", ")
        };

        let fallback_str = if info.fallback_dns.is_empty() {
            "None".to_string()
        } else {
            info.fallback_dns.join(", ")
        };

        let search_str = if info.search_domains.is_empty() {
            "None".to_string()
        } else {
            info.search_domains.join(", ")
        };

        let rows = vec![
            Row::new(vec!["Current DNS", &dns_str]),
            Row::new(vec!["Fallback DNS", &fallback_str]),
            Row::new(vec!["DNSSEC", &info.dnssec]),
            Row::new(vec!["DNS over TLS", &info.dnsovertls]),
            Row::new(vec!["Search Domains", &search_str]),
        ];

        let table = Table::new(rows, vec![Constraint::Length(16), Constraint::Min(40)]).block(block);

        f.render_widget(table, area);
    } else {
        let loading = Paragraph::new("Loading...").block(block);
        f.render_widget(loading, area);
    }
}

fn draw_interface_dns(ctx: &DnsContext, f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Per-Interface DNS ")
        .borders(Borders::ALL);

    if let Some(ref info) = ctx.info {
        if info.interface_dns.is_empty() {
            let empty = Paragraph::new("No interface-specific DNS configuration").block(block);
            f.render_widget(empty, area);
            return;
        }

        let header = Row::new(vec!["Interface", "DNS Servers", "Search Domains"])
            .style(Style::default().add_modifier(Modifier::BOLD));

        let rows: Vec<Row> = info
            .interface_dns
            .iter()
            .enumerate()
            .map(|(i, iface)| {
                let name_style = if i == ctx.selected_interface {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Cyan)
                };

                Row::new(vec![
                    Cell::from(iface.name.clone()).style(name_style),
                    Cell::from(iface.dns_servers.join(", ")),
                    Cell::from(iface.search_domains.join(", ")),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            vec![Constraint::Length(16), Constraint::Length(30), Constraint::Min(20)],
        )
        .header(header)
        .block(block);

        f.render_widget(table, area);
    } else {
        let loading = Paragraph::new("Loading...").block(block);
        f.render_widget(loading, area);
    }
}
