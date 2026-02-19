use anyhow::Result;
use zbus::{Connection, proxy};

/// Detect if running as root
pub fn is_root() -> bool {
    unsafe { libc::getuid() == 0 }
}

/// Systemd Manager D-Bus proxy
#[proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait SystemdManager {
    /// List all units
    /// Returns: [(name, description, load_state, active_state, sub_state,
    ///           follower, object_path, job_id, job_type, job_object_path)]
    fn list_units(
        &self,
    ) -> zbus::Result<
        Vec<(
            String,
            String,
            String,
            String,
            String,
            String,
            zbus::zvariant::OwnedObjectPath,
            u32,
            String,
            zbus::zvariant::OwnedObjectPath,
        )>,
    >;

    /// Get unit by name
    fn get_unit(&self, name: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    /// Start a unit
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    /// Stop a unit
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    /// Restart a unit
    fn restart_unit(&self, name: &str, mode: &str)
    -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    /// Reload daemon
    fn reload(&self) -> zbus::Result<()>;

    /// Enable unit files
    fn enable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
        force: bool,
    ) -> zbus::Result<(bool, Vec<(String, String, String)>)>;

    /// Disable unit files
    fn disable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
    ) -> zbus::Result<Vec<(String, String, String)>>;
}

#[derive(Clone)]
pub struct SystemdClient {
    connection: Connection,
    user_mode: bool,
}

impl SystemdClient {
    pub async fn new() -> Result<Self> {
        let (connection, user_mode) = if is_root() {
            // Running as root - connect to system bus
            let conn = Connection::system().await?;
            tracing::info!("Connected to system D-Bus as root");
            (conn, false)
        } else {
            // Not root - try user session first
            match Connection::session().await {
                Ok(conn) => {
                    tracing::info!("Connected to user D-Bus session");
                    (conn, true)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to connect to user session: {}, trying system bus",
                        e
                    );
                    let conn = Connection::system().await?;
                    tracing::info!("Connected to system D-Bus (read-only for non-root)");
                    (conn, false)
                }
            }
        };

        Ok(Self {
            connection,
            user_mode,
        })
    }

    pub fn is_user_mode(&self) -> bool {
        self.user_mode
    }

    /// Get the manager proxy for making calls
    async fn manager(&self) -> Result<SystemdManagerProxy<'_>> {
        let proxy = SystemdManagerProxy::new(&self.connection).await?;
        Ok(proxy)
    }

    /// List all units
    pub async fn list_units(&self) -> Result<Vec<UnitInfo>> {
        let manager = self.manager().await?;
        let units = manager.list_units().await?;

        let unit_info: Vec<UnitInfo> = units
            .into_iter()
            .map(
                |(name, description, load_state, active_state, sub_state, _, _, _, _, _)| {
                    UnitInfo {
                        name,
                        description,
                        load_state,
                        active_state,
                        sub_state,
                    }
                },
            )
            .collect();

        Ok(unit_info)
    }

    /// Start a unit
    pub async fn start_unit(&self, name: &str) -> Result<()> {
        let manager = self.manager().await?;
        let _job = manager.start_unit(name, "replace").await?;
        Ok(())
    }

    /// Stop a unit
    pub async fn stop_unit(&self, name: &str) -> Result<()> {
        let manager = self.manager().await?;
        let _job = manager.stop_unit(name, "replace").await?;
        Ok(())
    }

    /// Restart a unit
    pub async fn restart_unit(&self, name: &str) -> Result<()> {
        let manager = self.manager().await?;
        let _job = manager.restart_unit(name, "replace").await?;
        Ok(())
    }

    /// Reload daemon
    pub async fn reload_daemon(&self) -> Result<()> {
        let manager = self.manager().await?;
        manager.reload().await?;
        Ok(())
    }

    /// Enable a unit file
    pub async fn enable_unit(&self, name: &str) -> Result<()> {
        let manager = self.manager().await?;
        let _ = manager.enable_unit_files(&[name], false, true).await?;
        Ok(())
    }

    /// Disable a unit file
    pub async fn disable_unit(&self, name: &str) -> Result<()> {
        let manager = self.manager().await?;
        let _ = manager.disable_unit_files(&[name], false).await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct UnitInfo {
    pub name: String,
    pub description: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
}

impl UnitInfo {
    /// Check if unit is active
    pub fn is_active(&self) -> bool {
        self.active_state == "active"
    }

    /// Check if unit failed
    pub fn is_failed(&self) -> bool {
        self.active_state == "failed" || self.load_state == "error"
    }

    /// Get state icon/color indicator
    pub fn state_indicator(&self) -> &'static str {
        match self.active_state.as_str() {
            "active" => "●",
            "inactive" => "○",
            "failed" => "✗",
            "activating" => "◐",
            "deactivating" => "◑",
            _ => "?",
        }
    }
}
