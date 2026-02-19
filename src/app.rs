use crate::contexts::{
    boot::BootContext,
    dns::DnsContext,
    host::HostContext,
    logs::LogsContext,
    network::NetworkContext,
    units::UnitsContext,
    Context,
};
use crate::systemd::client::SystemdClient;
use anyhow::Result;
use crossterm::event::KeyEvent;

pub struct App {
    current_context: usize,
    show_help: bool,
    systemd: SystemdClient,
    units: UnitsContext,
    network: NetworkContext,
    dns: DnsContext,
    host: HostContext,
    boot: BootContext,
    logs: LogsContext,
    error_message: Option<String>,
}

impl App {
    pub async fn new() -> Result<Self> {
        let systemd = SystemdClient::new().await?;

        let units = UnitsContext::new(&systemd).await?;
        let network = NetworkContext::new();
        let dns = DnsContext::new();
        let host = HostContext::new();
        let boot = BootContext::new();
        let logs = LogsContext::new();

        Ok(Self {
            current_context: 0,
            show_help: false,
            systemd,
            units,
            network,
            dns,
            host,
            boot,
            logs,
            error_message: None,
        })
    }

    pub fn current_context(&self) -> usize {
        self.current_context
    }

    pub fn context_name(&self) -> &'static str {
        match self.current_context {
            0 => "Units",
            1 => "Network",
            2 => "DNS",
            3 => "Host",
            4 => "Boot",
            5 => "Logs",
            _ => "Unknown",
        }
    }

    pub fn next_context(&mut self) {
        self.current_context = (self.current_context + 1) % 6;
    }

    pub fn prev_context(&mut self) {
        if self.current_context == 0 {
            self.current_context = 5;
        } else {
            self.current_context -= 1;
        }
    }

    pub fn set_context(&mut self,
        ctx: usize,
    ) {
        if ctx < 6 {
            self.current_context = ctx;
        }
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn show_help(&self) -> bool {
        self.show_help
    }

    pub fn handle_key(&mut self,
        key: KeyEvent,
    ) {
        if self.show_help {
            // Any key closes help
            self.show_help = false;
            return;
        }

        // Route to current context
        match self.current_context {
            0 => self.units.handle_key(key),
            1 => self.network.handle_key(key),
            2 => self.dns.handle_key(key),
            3 => self.host.handle_key(key),
            4 => self.boot.handle_key(key),
            5 => self.logs.handle_key(key),
            _ => {}
        }
    }

    pub async fn tick(&mut self) {
        // Update current context
        match self.current_context {
            0 => self.units.tick().await,
            1 => self.network.tick().await,
            2 => self.dns.tick().await,
            3 => self.host.tick().await,
            4 => self.boot.tick().await,
            5 => self.logs.tick().await,
            _ => {}
        }
    }

    // Getters for contexts
    pub fn units(&self) -> &UnitsContext {
        &self.units
    }

    pub fn network(&self) -> &NetworkContext {
        &self.network
    }

    pub fn dns(&self) -> &DnsContext {
        &self.dns
    }

    pub fn host(&self) -> &HostContext {
        &self.host
    }

    pub fn boot(&self) -> &BootContext {
        &self.boot
    }

    pub fn logs(&self) -> &LogsContext {
        &self.logs
    }

    pub fn systemd(&self) -> &SystemdClient {
        &self.systemd
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    pub fn set_error(&mut self,
        msg: String,
    ) {
        self.error_message = Some(msg);
    }

    pub fn clear_error(&mut self,
    ) {
        self.error_message = None;
    }
}
