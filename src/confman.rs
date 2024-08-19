use core::panic;
use log::{log, Level};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json;
use std::{
    borrow::BorrowMut,
    error::Error,
    fs,
    io::{Read, Write},
    marker::PhantomData,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock, Weak},
};
use zbus::interface;

use crate::progbase;

struct ConfManClient<T> {
    storage: Weak<FileStorage<T>>,
    log_key: String,
}

impl<T> ConfManClient<T> {
    pub fn new(storage: Weak<FileStorage<T>>, key: &str) -> Self {
        Self {
            storage,
            log_key: key.to_string(),
        }
    }
}
#[interface(name = "is.centroid.Config")]
impl<T: Serialize + for<'de> Deserialize<'de> + JsonSchema + Default + Send + Sync + 'static>
    ConfManClient<T>
{
    #[zbus(property)]
    async fn value(&self) -> Result<String, zbus::fdo::Error> {
        let storage = self.storage.upgrade().unwrap();
        storage.to_json().map_err(|e| {
            let err_msg = format!("Error serializing to JSON: {}", e);
            log!(target: &self.log_key, Level::Error, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })
    }
    //busctl --user set-property is.centroid.framework-rs.def /is/centroid/Config/greeter is.centroid.Config Value s "new_value"
    //Failed to set property Value on interface is.centroid.Config: Failed to deserialize JSON: new_value. Error: expected ident at line 1 column 2
    #[zbus(property)]
    async fn set_value(&mut self, new_value: String) -> Result<(), zbus::fdo::Error> {
        let deserialized = serde_json::from_str(new_value.as_str());
        if let Err(e) = deserialized {
            let err_msg = format!("Failed to deserialize JSON: {}. Error: {}", new_value, e);
            log!(target: &self.log_key, Level::Error, "{}", err_msg);
            return Err(zbus::fdo::Error::InvalidArgs(err_msg));
        }
        let foo = deserialized.unwrap();
        let mut storage = self.storage.upgrade().unwrap();
        Arc::get_mut(&mut storage).unwrap().value = foo;
        Ok(())
    }
    #[zbus(property)]
    async fn schema(&self) -> Result<String, zbus::fdo::Error> {
        let storage = self.storage.upgrade().unwrap();
        storage.schema().map_err(|e| {
            let err_msg = format!("Error serializing to JSON schema: {}", e);
            log!(target: &self.log_key, Level::Error, "{}", err_msg);
            zbus::fdo::Error::Failed(err_msg)
        })
    }
}

pub struct ConfMan<T> {
    storage: Arc<FileStorage<T>>,
    log_key: String,
}

impl<
        'a,
        T: Serialize + for<'de> Deserialize<'de> + JsonSchema + Default + Send + Sync + 'static,
    > ConfMan<T>
{
    pub fn new(bus: zbus::Connection, key: &str) -> Self {
        let storage = Arc::new(FileStorage::<T>::new(&progbase::make_config_file_name(
            key, "json",
        )));
        let client = ConfManClient::new(Arc::downgrade(&storage), key);
        let path = format!("/is/centroid/Config/{}", key);
        tokio::spawn(async move {
            // log if error
            let _ =
                bus.object_server().at(path, client).await.map_err(
                    |e| log!(target: "ZBUS", Level::Error, "Error registering object: {}", e),
                );
        });

        ConfMan {
            storage,
            log_key: key.to_string(),
        }
    }

    pub fn value(&self) -> &T {
        &self.storage.value
    }

    pub fn make_change(&mut self) -> Change<Self, T> {
        Change::new(self)
    }

    pub fn to_json(&self) -> Result<String, Box<dyn Error>> {
        self.storage.to_json()
    }

    pub fn from_json(&mut self, value: &str) -> Result<(), Box<dyn Error>> {
        // todo can we do this more safely and still keep the same interface
        let deserialized_value: T = serde_json::from_str(value)?;
        Arc::get_mut(&mut self.storage).unwrap().value = deserialized_value;
        // *self.storage.make_change().value_mut() = deserialized_value;
        Ok(())
    }

    pub fn schema(&self) -> Result<String, Box<dyn Error>> {
        self.storage.schema()
    }

    pub fn file(&self) -> &PathBuf {
        self.storage.file()
    }
}

impl<T: for<'de> Deserialize<'de> + Serialize + JsonSchema + Default> ChangeTrait<T>
    for ConfMan<T>
{
    fn set_changed(&mut self) -> Result<(), Box<dyn Error>> {
        // todo can we do this more safely and still keep the same interface
        Arc::get_mut(&mut self.storage).unwrap().set_changed()
        // self.storage.set_changed()
    }
    fn value(&self) -> &T {
        self.storage.value()
    }
    fn value_mut(&mut self) -> &mut T {
        // todo can we do this more safely and still keep the same interface
        Arc::get_mut(&mut self.storage).unwrap().value_mut()
        // self.storage.value_mut()
    }
    fn key(&self) -> &str {
        &self.log_key
    }
}

trait ChangeTrait<T> {
    fn set_changed(&mut self) -> Result<(), Box<dyn Error>>;
    fn value(&self) -> &T;
    fn value_mut(&mut self) -> &mut T;
    fn key(&self) -> &str;
}

struct Change<'a, OwnerT, T>
where
    OwnerT: ChangeTrait<T>,
{
    owner: &'a mut OwnerT,
    _marker: PhantomData<T>,
}

impl<'a, OwnerT, T> Change<'a, OwnerT, T>
where
    OwnerT: ChangeTrait<T>,
{
    fn new(owner: &'a mut OwnerT) -> Self {
        Self {
            owner,
            _marker: PhantomData,
        }
    }
    fn value_mut(&mut self) -> &mut T {
        return self.owner.value_mut();
    }
}

impl<'a, OwnerT, T> Drop for Change<'a, OwnerT, T>
where
    OwnerT: ChangeTrait<T>,
{
    fn drop(&mut self) {
        if let Err(e) = self.owner.set_changed() {
            log!(target: self.owner.key(), Level::Warn,  "Error changing value: {}", e);
        }
    }
}

impl<'a, OwnerT, T> Deref for Change<'a, OwnerT, T>
where
    OwnerT: ChangeTrait<T>,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.owner.value()
    }
}

impl<'a, OwnerT, T> DerefMut for Change<'a, OwnerT, T>
where
    OwnerT: ChangeTrait<T>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.owner.value_mut()
    }
}

struct FileStorage<T> {
    value: T,
    filename: PathBuf,
    log_key: String,
}

impl<T: for<'de> Deserialize<'de> + Serialize + JsonSchema + Default> FileStorage<T> {
    fn new(path: &PathBuf) -> Self {
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                panic!(
                    "Error: {} Failed to create directories: {}",
                    e,
                    parent.display()
                );
            }
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .unwrap_or_else(|e| {
                panic!(
                    "Error: {} Failed to create or open file: {}",
                    e,
                    path.display()
                )
            });
        let mut file_content = String::new();
        let _ = file.read_to_string(&mut file_content);
        if file_content.is_empty() {
            // empty file let's create default constructed
            let default_value = T::default();
            file_content = serde_json::to_string(&default_value)
                .unwrap_or_else(|e| panic!("Error: \"{}\" Failed to serialize", e));
            file.write_all(file_content.as_bytes())
                .unwrap_or_else(|e| panic!("Error: \"{}\" Failed to write default value", e));
            let _ = file.flush();
        }
        let deserialized_value: T = serde_json::from_str(&file_content).unwrap_or_else(|e| {
            panic!(
                "Error: {} Failed to parse file contents: {}",
                e, &file_content
            )
        });

        FileStorage {
            value: deserialized_value,
            filename: path.clone(),
            log_key: path.to_str().unwrap().to_string(),
        }
    }

    fn to_json(&self) -> Result<String, Box<dyn Error>> {
        serde_json::to_string(&self.value).map_err(|e| {
            log!(target: &self.log_key, Level::Warn,  "Error serializing to JSON: {}", e);
            Box::new(e) as Box<dyn Error>
        })
    }

    fn schema(&self) -> Result<String, Box<dyn Error>> {
        serde_json::to_string_pretty(&schemars::schema_for!(T)).map_err(|e| {
            log!(target: &self.log_key, Level::Warn,  "Error serializing to JSON Schema: {}", e);
            Box::new(e) as Box<dyn Error>
        })
    }

    fn file(&self) -> &PathBuf {
        &self.filename
    }

    fn value(&self) -> &T {
        &self.value
    }

    fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    fn save_to_file(&self) -> Result<(), Box<dyn Error>> {
        let tmp_path = self.file().with_extension("tmp");

        let mut tmp_file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true) // Truncate to ensure it's empty
            .open(&tmp_path)
            .unwrap_or_else(|e| {
                panic!(
                    "Error: {} Failed to create or open temporary file: {}",
                    e,
                    tmp_path.display()
                )
            });

        tmp_file
            .write_all(self.to_json()?.as_bytes())
            .map_err(|e| Box::new(e) as Box<dyn Error>)?;

        tmp_file
            .flush()
            .map_err(|e| Box::new(e) as Box<dyn Error>)?;

        // move the temporary file to the target file
        std::fs::rename(&tmp_path, self.file()).map_err(|e| Box::new(e) as Box<dyn Error>)?;

        Ok(())
    }

    fn make_change(&mut self) -> Change<Self, T> {
        Change::new(self)
    }
}

impl<T: for<'de> Deserialize<'de> + Serialize + JsonSchema + Default> ChangeTrait<T>
    for FileStorage<T>
{
    fn set_changed(&mut self) -> Result<(), Box<dyn Error>> {
        self.save_to_file()
    }
    fn value(&self) -> &T {
        self.value()
    }
    fn value_mut(&mut self) -> &mut T {
        self.value_mut()
    }
    fn key(&self) -> &str {
        self.file().to_str().unwrap()
    }
}

#[cfg(test)]

mod tests {
    use super::ConfMan;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, Serialize, JsonSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct MyStruct {
        #[serde(rename = "myNumber")]
        pub my_int: i32,
        pub my_bool: bool,
        #[serde(default)]
        pub my_nullable_enum: Option<MyEnum>,
    }
    impl Default for MyStruct {
        fn default() -> Self {
            MyStruct {
                my_int: 5,
                my_bool: true,
                my_nullable_enum: None,
            }
        }
    }

    #[derive(Deserialize, Serialize, JsonSchema)]
    #[serde(untagged)]
    enum MyEnum {
        StringNewType(String),
        StructVariant { floats: Vec<f32> },
    }

    fn setup() {
        let current_dir = std::env::current_dir().expect("Failed to get current directory");
        let current_dir_str = current_dir
            .to_str()
            .expect("Failed to convert path to string");
        std::env::set_var("CONFIGURATION_DIRECTORY", current_dir_str);
    }
    #[test]
    fn struct_test() {
        setup();
        let config: ConfMan<MyStruct> = ConfMan::new("key");
        assert_eq!(config.value().my_int, 5);
    }

    #[test]
    fn change_param() {
        setup();
        let mut config: ConfMan<MyStruct> = ConfMan::new("key");
        config.make_change().my_int = 42; // Using DerefMut, this is pretty hidden feature but shortens code
        assert_eq!(config.value().my_int, 42);
    }
}
