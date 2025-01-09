use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tfc::progbase;
use tokio::sync::Mutex;
use zbus::interface;

pub static DBUS_PATH: &str = "/is/centroid/ipc_ruler";
pub static DBUS_SERVICE: &str = "is.centroid.ipc_ruler";
static LOG_KEY: &str = "ipc-ruler";
const SQLITE_BEHAVE: &str = "PRAGMA foreign_keys = ON;";
const SIGNALS_CREATE: &str = "CREATE TABLE IF NOT EXISTS signals(
              name TEXT,
              type INT,
              created_by TEXT,
              created_at LONG INTEGER,
              time_point_t LONG INTEGER,
              last_registered LONG INTEGER,
              description TEXT
            );";
const SLOTS_CREATE: &str = "CREATE TABLE IF NOT EXISTS slots(
              name TEXT,
              type INT,
              created_by TEXT,
              created_at LONG INTEGER,
              last_registered LONG INTEGER,
              last_modified INTEGER,
              modified_by TEXT,
              connected_to TEXT,
              time_point_t LONG INTEGER,
              description TEXT
            );";

#[derive(Debug, Serialize, Deserialize)]
pub struct SignalRecord {
    name: String,
    #[serde(rename = "type")]
    sig_type: u8,
    created_by: String,
    created_at: u64,
    last_registered: u64,
    description: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SlotRecord {
    name: String,
    #[serde(rename = "type")]
    slot_type: u8,
    created_by: String,
    created_at: u64,
    last_registered: u64,
    last_modified: u64,
    modified_by: String,
    connected_to: String,
    description: String,
}

pub struct ConnectionChange {
    slot_name: String,
    signal_name: String,
}

pub struct IpcRuler {
    db: Arc<Mutex<rusqlite::Connection>>,
}

impl IpcRuler {
    pub fn spawn(dbus: zbus::Connection, in_memory: bool) -> tokio::task::JoinHandle<()> {
        let dbus_task = async move {
            let client = Self::new(in_memory);
            let res = dbus
                .object_server()
                .at(DBUS_PATH, client)
                .await
                .expect(&format!("Error registering object: {}", DBUS_PATH));
            if !res {
                panic!("Interface IpcRuler already registered at {}", DBUS_PATH);
            }
        };
        tokio::spawn(dbus_task)
    }
    pub fn new(in_memory: bool) -> Self {
        let connection: rusqlite::Connection = match in_memory {
            true => {
                rusqlite::Connection::open_in_memory().expect("Failed to open in-memory database")
            }
            false => {
                let db_path = progbase::make_config_file_name("ipc-ruler", "db");
                if !db_path.parent().unwrap().exists() {
                    std::fs::create_dir_all(db_path.parent().unwrap())
                        .expect("Failed to create db directory");
                }
                rusqlite::Connection::open(db_path.clone()).expect(&format!(
                    "Failed to open or create db: {}",
                    db_path.display()
                ))
            }
        };
        connection
            .execute(SQLITE_BEHAVE, ())
            .expect("Sqlite set foreign key references");
        connection
            .execute(SIGNALS_CREATE, ())
            .expect("Create signals table");
        connection
            .execute(SLOTS_CREATE, ())
            .expect("Create slots table");

        Self {
            db: Arc::new(Mutex::new(connection)),
        }
    }

    async fn connections_impl(&self) -> Result<String, rusqlite::Error> {
        debug!(target: LOG_KEY, "connections called");
        let db = self.db.lock().await;
        let mut stmt = db.prepare(
            "SELECT signals.name, slots.name FROM signals JOIN slots ON signals.name = slots.connected_to;"
        )?;

        let mut connections: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        for row in rows {
            let (signal_name, slot_name) = row?;
            connections.entry(signal_name).or_default().push(slot_name);
        }

        Ok(serde_json::to_string(&connections).expect("Failed to serialize connections"))
    }

    async fn signals_impl(&self) -> Result<String, rusqlite::Error> {
        debug!(target: LOG_KEY, "signals called");
        let db = self.db.lock().await;
        let mut stmt = db.prepare(
            "SELECT name, type, last_registered, description, created_at, created_by FROM signals",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SignalRecord {
                    name: row.get(0)?,
                    sig_type: row.get(1)?,
                    last_registered: row.get(2)?,
                    description: row.get(3)?,
                    created_at: row.get(4)?,
                    created_by: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(serde_json::to_string(&rows).expect("Failed to serialize signals"))
    }

    async fn slots_impl(&self) -> Result<String, rusqlite::Error> {
        debug!(target: LOG_KEY, "slots called");
        let db = self.db.lock().await;
        let mut stmt = db.prepare(
            "SELECT name, type, last_registered, description, created_at, created_by, last_modified, modified_by, connected_to FROM slots",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SlotRecord {
                    name: row.get(0)?,
                    slot_type: row.get(1)?,
                    last_registered: row.get(2)?,
                    description: row.get(3)?,
                    created_at: row.get(4)?,
                    created_by: row.get(5)?,
                    last_modified: row.get(6)?,
                    modified_by: row.get(7).unwrap_or("".to_string()),
                    connected_to: row.get(8).unwrap_or("".to_string()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(serde_json::to_string(&rows).expect("Failed to serialize slots"))
    }

    async fn connect_impl(&self, change: ConnectionChange) -> Result<(), rusqlite::Error> {
        debug!(target: LOG_KEY,
            "connect called, slot: {}, signal: {}",
            change.slot_name, change.signal_name
        );
        let db = self.db.lock().await;
        let slot_count: i64 = db.query_row(
            "SELECT count(*) FROM slots WHERE name = ?;",
            [&change.slot_name],
            |row| row.get(0),
        )?;
        if slot_count == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }

        // Skip signal validation if we're disconnecting (empty signal name)
        if !change.signal_name.is_empty() {
            let signal_count: i64 = db.query_row(
                "SELECT count(*) FROM signals WHERE name = ?;",
                [&change.signal_name],
                |row| row.get(0),
            )?;
            if signal_count == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let signal_type: u8 = db.query_row(
                "SELECT type FROM signals WHERE name = ?;",
                [&change.signal_name],
                |row| row.get(0),
            )?;
            let slot_type: u8 = db.query_row(
                "SELECT type FROM slots WHERE name = ?;",
                [&change.slot_name],
                |row| row.get(0),
            )?;
            if signal_type != slot_type {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
        }

        db.execute(
            "UPDATE slots SET connected_to = ? WHERE name = ?;",
            rusqlite::params![change.signal_name, change.slot_name],
        )?;

        Ok(())
    }
    async fn register_signal_impl(&self, signal: SignalRecord) -> Result<(), rusqlite::Error> {
        debug!(target: LOG_KEY, "register_signal called, signal: {:?}", signal);
        let db = self.db.lock().await;
        let count: i64 = db.query_row(
            "SELECT count(*) FROM signals WHERE name = ?;",
            [&signal.name],
            |row| row.get(0),
        )?;
        if count != 0 {
            // update the signal
            db.execute(
                "UPDATE signals SET last_registered = ?, description = ?, type = ?, created_by = ? WHERE name = ?;",
                rusqlite::params![
                    signal.last_registered,
                    signal.description,
                    signal.sig_type,
                    signal.created_by,
                    signal.name
                ],
            )?;
        } else {
            // Insert the signal
            db.execute(
                "INSERT INTO signals (name, type, created_by, created_at, last_registered, description) VALUES (?, ?, ?, ?, ?, ?);",
                rusqlite::params![
                    signal.name,
                    signal.sig_type,
                    signal.created_by,
                    signal.created_at,
                    signal.last_registered,
                    signal.description
                ],
            )?;
        }
        Ok(())
    }

    async fn register_slot_impl(&self, slot: SlotRecord) -> Result<String, rusqlite::Error> {
        debug!(target: LOG_KEY, "register_slot called, slot: {:?}", slot);
        let db = self.db.lock().await;
        let mut connected_to = String::new();

        // Check if slot exists and get its connected_to value
        let count: i64 = db.query_row(
            "SELECT count(*) FROM slots WHERE name = ?;",
            [&slot.name],
            |row| row.get(0),
        )?;

        if count > 0 {
            // Update existing slot and handle null connected_to values
            connected_to = db.query_row(
                "SELECT COALESCE(connected_to, '') FROM slots WHERE name = ?;",
                [&slot.name],
                |row| row.get(0),
            )?;

            db.execute(
                "UPDATE slots SET last_registered = ?, description = ?, type = ?, created_by = ? WHERE name = ?;",
                rusqlite::params![
                    slot.last_registered,
                    slot.description,
                    slot.slot_type,
                    slot.created_by,
                    slot.name
                ],
            )?;
        } else {
            // Insert new slot
            db.execute(
                "INSERT INTO slots (name, type, created_by, created_at, last_registered, last_modified, description, connected_to) VALUES (?, ?, ?, ?, ?, ?, ?, ?);",
                rusqlite::params![
                    slot.name,
                    slot.slot_type,
                    slot.created_by,
                    slot.created_at,
                    slot.last_registered,
                    slot.last_modified,
                    slot.description,
                    "" // Initialize connected_to with empty string
                ],
            )?;
        }

        Ok(connected_to)
    }
}

#[interface(name = "is.centroid.manager")]
impl IpcRuler {
    #[zbus(property)]
    async fn connections(&self) -> zbus::fdo::Result<String> {
        self.connections_impl().await.map_err(|e| {
            error!(target: LOG_KEY, "connections poll failed, error: {}", e);
            zbus::fdo::Error::Failed(e.to_string())
        })
    }

    #[zbus(property)]
    async fn signals(&self) -> zbus::fdo::Result<String> {
        self.signals_impl().await.map_err(|e| {
            error!(target: LOG_KEY, "signals poll failed, error: {}", e);
            zbus::fdo::Error::Failed(e.to_string())
        })
    }

    #[zbus(property)]
    async fn slots(&self) -> zbus::fdo::Result<String> {
        self.slots_impl().await.map_err(|e| {
            error!(target: LOG_KEY, "slots poll failed, error: {}", e);
            zbus::fdo::Error::Failed(e.to_string())
        })
    }

    async fn connect(
        &self,
        #[zbus(signal_context)] ctxt: zbus::SignalContext<'_>,
        slot_name: String,
        signal_name: String,
    ) -> zbus::fdo::Result<()> {
        debug!(target: LOG_KEY,
            "connect called, slot_name: {}, signal_name: {}",
            slot_name, signal_name
        );
        self.connect_impl(ConnectionChange {
            slot_name: slot_name.clone(),
            signal_name: signal_name.clone(),
        })
        .await
        .map_err(|e| {
            error!(target: LOG_KEY, "connect failed, error: {}", e);
            zbus::fdo::Error::Failed(e.to_string())
        })?;
        IpcRuler::connection_change(&ctxt, &slot_name, &signal_name).await?;
        Ok(())
    }

    async fn disconnect(
        &self,
        #[zbus(signal_context)] ctxt: zbus::SignalContext<'_>,
        slot_name: String,
    ) -> zbus::fdo::Result<()> {
        debug!(target: LOG_KEY, "disconnect called, slot_name: {}", slot_name);
        self.connect(ctxt, slot_name, "".to_string()).await?;
        Ok(())
    }

    async fn register_signal(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        name: String,
        description: String,
        type_id: u8,
    ) -> zbus::fdo::Result<()> {
        debug!(target: LOG_KEY,
            "register_signal called, name: {}, description: {}, type_id: {}",
            name, description, type_id
        );
        let caller_id = hdr
            .sender()
            .map_or("unknown".to_string(), |id| id.to_string());
        let timestamp_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        self.register_signal_impl(SignalRecord {
            name,
            sig_type: type_id,
            created_by: caller_id,
            created_at: timestamp_now,
            last_registered: timestamp_now,
            description,
        })
        .await
        .map_err(|e| {
            error!(target: LOG_KEY, "register_signal failed, error: {}", e);
            zbus::fdo::Error::Failed(e.to_string())
        })?;

        Ok(())
    }

    async fn register_slot(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(signal_context)] ctxt: zbus::SignalContext<'_>,
        name: String,
        description: String,
        type_id: u8,
    ) -> zbus::fdo::Result<()> {
        debug!(target: LOG_KEY,
            "register_slot called, name: {}, description: {}, type_id: {}",
            name, description, type_id
        );
        let caller_id = hdr
            .sender()
            .map_or("unknown".to_string(), |id| id.to_string());
        let timestamp_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let connected_to = self
            .register_slot_impl(SlotRecord {
                name: name.clone(),
                slot_type: type_id,
                created_by: caller_id,
                created_at: timestamp_now,
                last_registered: timestamp_now,
                last_modified: timestamp_now,
                modified_by: "".to_string(),
                connected_to: "".to_string(),
                description,
            })
            .await
            .map_err(|e| {
                error!(target: LOG_KEY, "register_slot failed, error: {}", e);
                zbus::fdo::Error::Failed(e.to_string())
            })?;
        IpcRuler::connection_change(&ctxt, &name, &connected_to).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn connection_change(
        signal_ctxt: &zbus::SignalContext<'_>,
        slot_name: &str,
        signal_name: &str,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::trace;
    use tfc::ipc_ruler_client::IpcRulerProxy;

    struct Test {
        bus: zbus::Connection,
        handle: tokio::task::JoinHandle<()>,
        proxy: IpcRulerProxy<'static>,
    }

    impl Test {
        async fn new() -> Self {
            tfc::logger::init_test_logger(log::LevelFilter::Trace).expect("Failed to init logger");
            let bus = zbus::connection::Builder::system()
                .expect("zbus builder")
                .name(DBUS_SERVICE)
                .expect("zbus name")
                .build()
                .await
                .expect("Failed to create zbus");
            Self {
                bus: bus.clone(),
                handle: IpcRuler::spawn(bus.clone(), true),
                proxy: IpcRulerProxy::builder(&bus)
                    .cache_properties(zbus::CacheProperties::No)
                    .build()
                    .await
                    .expect("Failed to create ipc ruler proxy"),
            }
        }
        async fn register_signal(
            &self,
            name: &str,
            description: &str,
            type_id: u8,
        ) -> Result<(), Box<dyn std::error::Error>> {
            let mut i = 0;
            while i < 10 {
                let res = tokio::time::timeout(
                    tokio::time::Duration::from_millis(1),
                    self.proxy.register_signal(name, description, type_id),
                )
                .await;
                if res.is_ok() {
                    if res.unwrap().is_ok() {
                        return Ok(());
                    }
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Failed to register signal",
                    )));
                }
                trace!("res: {:?}", res);
                i += 1;
            }
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to register signal timeout",
            )))
        }
        async fn register_slot(
            &self,
            name: &str,
            description: &str,
            type_id: u8,
        ) -> Result<(), Box<dyn std::error::Error>> {
            let mut i = 0;
            while i < 10 {
                let res = tokio::time::timeout(
                    tokio::time::Duration::from_millis(1),
                    self.proxy.register_slot(name, description, type_id),
                )
                .await;
                if res.is_ok() {
                    if res.unwrap().is_ok() {
                        return Ok(());
                    }
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Failed to register slot",
                    )));
                }
                trace!("res: {:?}", res);
                i += 1;
            }
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to register slot timeout",
            )))
        }
        async fn slots(&self) -> Result<Vec<SlotRecord>, Box<dyn std::error::Error>> {
            let slots = self.proxy.slots().await?;
            Ok(serde_json::from_str(&slots)?)
        }
        async fn signals(&self) -> Result<Vec<SignalRecord>, Box<dyn std::error::Error>> {
            let signals = self.proxy.signals().await?;
            Ok(serde_json::from_str(&signals)?)
        }
    }
    impl Drop for Test {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_register_signal() -> Result<(), Box<dyn std::error::Error>> {
        let test = Test::new().await;
        assert!(test
            .register_signal("test_signal", "test_description", 1)
            .await
            .is_ok());
        let signals = test.proxy.signals().await?;

        assert!(signals.contains("test_signal"));
        trace!("signals: {:?}", signals);

        Ok(())
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_register_slot() -> Result<(), Box<dyn std::error::Error>> {
        let test = Test::new().await;
        assert!(test
            .register_slot("test_slot", "test_description", 1)
            .await
            .is_ok());
        let slots = test.proxy.slots().await?;
        assert!(slots.contains("test_slot"));
        trace!("slots: {:?}", slots);
        Ok(())
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_slot_reregistration() -> Result<(), Box<dyn std::error::Error>> {
        let test = Test::new().await;
        assert!(test
            .register_slot("test_slot", "test_description", 1)
            .await
            .is_ok());
        let slots = test.slots().await?;
        assert!(slots.len() == 1);
        assert!(test
            .register_slot("test_slot", "test_description", 1)
            .await
            .is_ok());
        let slots = test.slots().await?;
        assert!(slots.len() == 1);
        Ok(())
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_reregister_signal() -> Result<(), Box<dyn std::error::Error>> {
        let test = Test::new().await;
        assert!(test
            .register_signal("test_signal", "test_description", 1)
            .await
            .is_ok());
        let signals = test.signals().await?;
        assert!(signals.len() == 1);
        assert!(test
            .register_signal("test_signal", "test_description", 1)
            .await
            .is_ok());
        let signals = test.signals().await?;
        assert!(signals.len() == 1);
        Ok(())
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_disconnect() -> Result<(), Box<dyn std::error::Error>> {
        let test = Test::new().await;
        assert!(test
            .register_slot("test_slot", "test_description", 1)
            .await
            .is_ok());
        assert!(test
            .register_signal("test_signal", "test_description", 1)
            .await
            .is_ok());
        test.proxy.connect("test_slot", "test_signal").await?;
        test.proxy.disconnect("test_slot").await?;
        let connections = test.proxy.connections().await?;
        assert!(!connections.contains("test_slot"));
        Ok(())
    }
}
