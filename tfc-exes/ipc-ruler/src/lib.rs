use log::debug;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tfc::progbase;
use tokio::sync::Mutex;
use zbus::interface;

pub static DBUS_PATH: &str = "/is/centroid/ipc_ruler";
pub static DBUS_SERVICE: &str = "is.centroid.ipc_ruler";
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

pub struct IpcRuler {
    db: rusqlite::Connection,
    connection_change_tx: tokio::sync::mpsc::Sender<ConnectionChange>,
}

impl IpcRuler {
    pub fn spawn(
        dbus: zbus::Connection,
        in_memory: bool,
    ) -> (Arc<Mutex<Self>>, tokio::task::JoinHandle<()>) {
        let (connections_tx, mut connections_rx) = tokio::sync::mpsc::channel(10);
        let (signals_tx, mut signals_rx) = tokio::sync::mpsc::channel(10);
        let (slots_tx, mut slots_rx) = tokio::sync::mpsc::channel(10);
        let (connect_sender, mut connect_rx) = tokio::sync::mpsc::channel(10);
        let (register_signal_sender, mut register_signal_rx) = tokio::sync::mpsc::channel(10);
        let (register_slot_sender, mut register_slot_rx) = tokio::sync::mpsc::channel(10);
        let (connection_change_tx, mut connection_change_rx) =
            tokio::sync::mpsc::channel::<ConnectionChange>(10);

        let this = Arc::new(Mutex::new(Self::new(connection_change_tx, in_memory)));
        let this_clone = this.clone();

        let dbus_task = async move {
            let client = IpcRulerDbusService::new(
                connections_tx,
                signals_tx,
                slots_tx,
                connect_sender,
                register_signal_sender,
                register_slot_sender,
            );
            let res = dbus
                .object_server()
                .at(DBUS_PATH, client)
                .await
                .expect(&format!("Error registering object: {}", DBUS_PATH));
            if !res {
                log::error!("Interface IpcRuler already registered at {}", DBUS_PATH);
            }
            let iface: zbus::InterfaceRef<IpcRulerDbusService> = loop {
                match dbus.object_server().interface(DBUS_PATH).await {
                    Ok(iface) => break iface,
                    Err(_) => {
                        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                        continue;
                    }
                }
            };
            loop {
                tokio::select! {
                    change = connection_change_rx.recv() => {
                        match change {
                            Some(change) => {
                                let _ = IpcRulerDbusService::connection_change(
                                    &iface.signal_context(),
                                    &change.slot_name,
                                    &change.signal_name,
                                ).await.map_err(|e| log::error!("Failed to send connection change: {:?}", e));
                            }
                            _ => {
                                panic!("Connection change channel closed");
                            }
                        }
                    }
                    connections_response_tx = connections_rx.recv() => {
                        match connections_response_tx {
                            Some(response_tx) => {
                                let _ = response_tx.send(this_clone.lock().await.connections().await).map_err(|e| log::error!("Failed to send connections response: {:?}", e));
                            }
                            _ => {
                                panic!("Connections response channel closed");
                            }
                        }
                    }
                    signals_response_tx = signals_rx.recv() => {
                        match signals_response_tx {
                            Some(response_tx) => {
                                let _ = response_tx.send(this_clone.lock().await.signals().await).map_err(|e| log::error!("Failed to send signals response: {:?}", e));
                            }
                            _ => {
                                panic!("Signals response channel closed");
                            }
                        }
                    }
                    slots_response_tx = slots_rx.recv() => {
                        match slots_response_tx {
                            Some(response_tx) => {
                                let _ = response_tx.send(this_clone.lock().await.slots().await).map_err(|e| log::error!("Failed to send slots response: {:?}", e));
                            }
                            _ => {
                                panic!("Slots response channel closed");
                            }
                        }
                    }
                    connect_response_tx = connect_rx.recv() => {
                        match connect_response_tx {
                            Some((change, response_tx)) => {
                                let _ = response_tx.send(this_clone.lock().await.connect(change).await).map_err(|e| log::error!("Failed to send connect response: {:?}", e));
                            }
                            _ => {
                                panic!("Connect response channel closed");
                            }
                        }
                    }
                    register_signal_response_tx = register_signal_rx.recv() => {
                        match register_signal_response_tx {
                            Some((signal, response_tx)) => {
                                let _ = response_tx.send(this_clone.lock().await.register_signal(signal).await).map_err(|e| log::error!("Failed to send register signal response: {:?}", e));
                            }
                            _ => {
                                panic!("Register signal response channel closed");
                            }
                        }
                    }
                    register_slot_response_tx = register_slot_rx.recv() => {
                        match register_slot_response_tx {
                            Some((slot, response_tx)) => {
                                let _ = response_tx.send(this_clone.lock().await.register_slot(slot).await).map_err(|e| log::error!("Failed to send register slot response: {:?}", e));
                            }
                            _ => {
                                panic!("Register slot response channel closed");
                            }
                        }
                    }
                }
            }
        };
        (this, tokio::spawn(dbus_task))
    }
    fn new(
        connection_change_tx: tokio::sync::mpsc::Sender<ConnectionChange>,
        in_memory: bool,
    ) -> Self {
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
            db: connection,
            connection_change_tx,
        }
    }

    pub async fn register_signal(&mut self, signal: SignalRecord) -> Result<(), rusqlite::Error> {
        debug!("register_signal called, signal: {:?}", signal);
        let count: i64 = self.db.query_row(
            "SELECT count(*) FROM signals WHERE name = ?;",
            [&signal.name],
            |row| row.get(0),
        )?;
        if count != 0 {
            // update the signal
            self.db.execute(
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
            self.db.execute(
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

    pub async fn register_slot(&mut self, slot: SlotRecord) -> Result<(), rusqlite::Error> {
        debug!("register_slot called, slot: {:?}", slot);
        let mut connected_to = String::new();
        let found = self
            .db
            .query_row(
                "SELECT connected_to FROM slots WHERE name = ?;",
                [&slot.name],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .map(|val| {
                connected_to = val;
                true
            })
            .unwrap_or(false);
        if found {
            // update the slot
            self.db.execute(
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
            // Insert the slot
            //   name TEXT,
            //   type INT,
            //   created_by TEXT,
            //   created_at LONG INTEGER,
            //   last_registered LONG INTEGER,
            //   last_modified INTEGER,
            //   modified_by TEXT,
            //   connected_to TEXT,
            //   time_point_t LONG INTEGER,
            //   description TEXT
            self.db.execute(
                "INSERT INTO slots (name, type, created_by, created_at, last_registered, last_modified, description) VALUES (?, ?, ?, ?, ?, ?, ?);",
                rusqlite::params![
                    slot.name,
                    slot.slot_type,
                    slot.created_by,
                    slot.created_at,
                    slot.last_registered,
                    slot.last_modified,
                    slot.description
                ],
            )?;
        }
        self.connection_change_tx
            .send(ConnectionChange {
                slot_name: slot.name,
                signal_name: slot.connected_to,
            })
            .await
            .expect("Failed to send connection change");
        Ok(())
    }
    pub async fn connect(&mut self, change: ConnectionChange) -> Result<(), rusqlite::Error> {
        debug!(
            "connect called, slot: {}, signal: {}",
            change.slot_name, change.signal_name
        );
        let slot_count: i64 = self.db.query_row(
            "SELECT count(*) FROM slots WHERE name = ?;",
            [&change.slot_name],
            |row| row.get(0),
        )?;
        if slot_count == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        let signal_count: i64 = self.db.query_row(
            "SELECT count(*) FROM signals WHERE name = ?;",
            [&change.signal_name],
            |row| row.get(0),
        )?;
        if signal_count == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        let signal_type: u8 = self.db.query_row(
            "SELECT type FROM signals WHERE name = ?;",
            [&change.signal_name],
            |row| row.get(0),
        )?;
        let slot_type: u8 = self.db.query_row(
            "SELECT type FROM slots WHERE name = ?;",
            [&change.slot_name],
            |row| row.get(0),
        )?;
        if signal_type != slot_type {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }

        self.db.execute(
            "UPDATE slots SET connected_to = ? WHERE name = ?;",
            rusqlite::params![change.signal_name, change.slot_name],
        )?;
        self.connection_change_tx
            .send(ConnectionChange {
                slot_name: change.slot_name,
                signal_name: change.signal_name,
            })
            .await
            .expect("Failed to send connection change");
        Ok(())
    }
    pub async fn connections(&mut self) -> Result<String, rusqlite::Error> {
        debug!("connections called");
        let mut stmt = self.db.prepare(
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
    pub async fn signals(&mut self) -> Result<String, rusqlite::Error> {
        debug!("signals called");
        let mut stmt = self.db.prepare(
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
    pub async fn slots(&mut self) -> Result<String, rusqlite::Error> {
        debug!("slots called");
        let mut stmt = self.db.prepare(
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
}

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

struct IpcRulerDbusService {
    connections:
        tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<String, rusqlite::Error>>>,
    signals:
        tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<String, rusqlite::Error>>>,
    slots: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<String, rusqlite::Error>>>,
    connect_sender: tokio::sync::mpsc::Sender<(
        ConnectionChange,
        tokio::sync::oneshot::Sender<Result<(), rusqlite::Error>>,
    )>,
    register_signal_sender: tokio::sync::mpsc::Sender<(
        SignalRecord,
        tokio::sync::oneshot::Sender<Result<(), rusqlite::Error>>,
    )>,
    register_slot_sender: tokio::sync::mpsc::Sender<(
        SlotRecord,
        tokio::sync::oneshot::Sender<Result<(), rusqlite::Error>>,
    )>,
}

impl IpcRulerDbusService {
    pub fn new(
        connections: tokio::sync::mpsc::Sender<
            tokio::sync::oneshot::Sender<Result<String, rusqlite::Error>>,
        >,
        signals: tokio::sync::mpsc::Sender<
            tokio::sync::oneshot::Sender<Result<String, rusqlite::Error>>,
        >,
        slots: tokio::sync::mpsc::Sender<
            tokio::sync::oneshot::Sender<Result<String, rusqlite::Error>>,
        >,
        connect_sender: tokio::sync::mpsc::Sender<(
            ConnectionChange,
            tokio::sync::oneshot::Sender<Result<(), rusqlite::Error>>,
        )>,
        register_signal_sender: tokio::sync::mpsc::Sender<(
            SignalRecord,
            tokio::sync::oneshot::Sender<Result<(), rusqlite::Error>>,
        )>,
        register_slot_sender: tokio::sync::mpsc::Sender<(
            SlotRecord,
            tokio::sync::oneshot::Sender<Result<(), rusqlite::Error>>,
        )>,
    ) -> Self {
        Self {
            connections,
            signals,
            slots,
            connect_sender,
            register_signal_sender,
            register_slot_sender,
        }
    }
}

#[interface(name = "is.centroid.manager")]
impl IpcRulerDbusService {
    #[zbus(property)]
    async fn connections(&self) -> zbus::fdo::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, rusqlite::Error>>();
        self.connections
            .send(tx)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let res = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        match res {
            Ok(res) => Ok(res),
            Err(e) => Err(zbus::fdo::Error::Failed(e.to_string())),
        }
    }

    #[zbus(property)]
    async fn signals(&self) -> zbus::fdo::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, rusqlite::Error>>();
        self.signals
            .send(tx)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let res = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        match res {
            Ok(res) => Ok(res),
            Err(e) => Err(zbus::fdo::Error::Failed(e.to_string())),
        }
    }

    #[zbus(property)]
    async fn slots(&self) -> zbus::fdo::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, rusqlite::Error>>();
        self.slots
            .send(tx)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let res = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        match res {
            Ok(res) => Ok(res),
            Err(e) => Err(zbus::fdo::Error::Failed(e.to_string())),
        }
    }

    async fn connect(&self, slot_name: String, signal_name: String) -> zbus::fdo::Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.connect_sender
            .send((
                ConnectionChange {
                    slot_name,
                    signal_name,
                },
                tx,
            ))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let _ = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn disconnect(&self, slot_name: String) -> zbus::fdo::Result<()> {
        self.connect(slot_name, "".to_string()).await?;
        Ok(())
    }

    async fn register_signal(
        &self,
        name: String,
        description: String,
        type_id: u8,
        // header: &zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        // let caller_id = header
        //     .sender()
        //    .map_or("unknown".to_string(), |id| id.to_string());
        let timestamp_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.register_signal_sender
            .send((
                SignalRecord {
                    name,
                    sig_type: type_id,
                    created_by: "".to_string(),
                    created_at: timestamp_now,
                    last_registered: timestamp_now,
                    description,
                },
                tx,
            ))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;

        let _ = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn register_slot(
        &self,
        name: String,
        description: String,
        type_id: u8,
    ) -> zbus::fdo::Result<()> {
        let timestamp_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.register_slot_sender
            .send((
                SlotRecord {
                    name,
                    slot_type: type_id,
                    created_by: "".to_string(),
                    created_at: timestamp_now,
                    last_registered: timestamp_now,
                    last_modified: timestamp_now,
                    modified_by: "".to_string(),
                    connected_to: "".to_string(),
                    description,
                },
                tx,
            ))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let _ = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
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

    #[tokio::test]
    #[serial_test::serial]
    async fn test_register_signal() -> Result<(), Box<dyn std::error::Error>> {
        tfc::logger::init_test_logger(log::LevelFilter::Trace)?;
        trace!("create bus");
        let bus = zbus::connection::Builder::system()?
            .name(DBUS_SERVICE)?
            .build()
            .await?;
        trace!("spawn ruler");
        let (_, handle) = IpcRuler::spawn(bus.clone(), true);
        trace!("create proxy");
        let proxy = IpcRulerProxy::builder(&bus)
            .cache_properties(zbus::CacheProperties::No)
            .build()
            .await
            .unwrap();
        trace!("register signal");
        let mut i = 0;
        while i < 10 {
            let res = tokio::time::timeout(
                tokio::time::Duration::from_millis(1),
                proxy.register_signal("test_signal", "test_description", 1),
            )
            .await;
            if res.is_ok() {
                break;
            }
            trace!("res: {:?}", res);
            i += 1;
        }
        let signals = proxy.signals().await?;

        assert!(signals.contains("test_signal"));
        trace!("signals: {:?}", signals);

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_register_slot() -> Result<(), Box<dyn std::error::Error>> {
        tfc::logger::init_test_logger(log::LevelFilter::Trace)?;
        trace!("create bus");
        let bus = zbus::connection::Builder::system()?
            .name(DBUS_SERVICE)?
            .build()
            .await?;
        trace!("spawn ruler");
        let (_, handle) = IpcRuler::spawn(bus.clone(), true);
        trace!("create proxy");
        let proxy = IpcRulerProxy::builder(&bus)
            .cache_properties(zbus::CacheProperties::No)
            .build()
            .await
            .unwrap();
        trace!("register slot");
        let mut i = 0;
        while i < 10 {
            let res = tokio::time::timeout(
                tokio::time::Duration::from_millis(1),
                proxy.register_slot("test_slot", "test_description", 1),
            )
            .await;
            if res.is_ok() {
                break;
            }
            i += 1;
        }
        let slots = proxy.slots().await?;
        assert!(slots.contains("test_slot"));
        trace!("slots: {:?}", slots);
        handle.abort();
        Ok(())
    }
}
