use core::panic;
use log::{log, Level};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json;
use std::{error::Error, fs, io::Read, path::PathBuf};

use crate::progbase;

struct FileStorage<T> {
    value: T,
    filename: PathBuf,
}

impl<T: for<'de> Deserialize<'de>> FileStorage<T> {
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
            .unwrap_or_else(|e| panic!("Error: {} Failed to create or open file: {}", e, path.display()));
        let mut file_content  = String::new();
        file.read_to_string(&mut file_content);
        let deserialized_value: T = serde_json::from_str(&file_content)
            .unwrap_or_else(|e| panic!("Error: {} Failed to parse file contents: {}", e, &file_content));


        FileStorage {
            value: deserialized_value,
            filename: path.clone(),
        }
    }

    fn file(&self) -> &PathBuf {
        &self.filename
    }

    fn set_value(&mut self, value: T) -> Result<(), Box<dyn Error>> {
        // todo
        self.value = value;
        Ok(())
    }
}

struct Confman<T> {
    storage: FileStorage<T>,
    key: String,
}

impl<T: Serialize + for<'de> Deserialize<'de> + JsonSchema + Default> Confman<T> {
    fn new(bus: zbus::Connection, key: &str) -> Self {
        Confman {
            storage: FileStorage::<T>::new(&progbase::make_config_file_name(key, "json")),
            key: key.to_string(),
        }
    }

    fn value(&self) -> &T {
        &self.storage.value
    }

    fn to_json_string(&self) -> Result<String, Box<dyn Error>> {
        serde_json::to_string(self.value()).map_err(|e| {
            log!(target: &self.key, Level::Warn,  "Error serializing to JSON: {}", e);
            Box::new(e) as Box<dyn Error>
        })
    }

    fn schema(&self) -> String {
        serde_json::to_string_pretty(&schemars::schema_for!(T)).unwrap()
    }

    fn set_changed(&self) -> Result<(), Box<dyn Error>> {
        let _ = self.to_json_string();
        // if Ok(json_str) {
        //     // self.client.set(json_str);
        //     // self.storage.set_changed()
        //     Ok(())
        // } else {
        //     Err(json_str)
        // }
        Ok(())
    }

    fn from_string(&mut self, value: &str) -> Result<(), Box<dyn Error>> {
        let deserialized_value: T = serde_json::from_str(value)?;
        self.storage.set_value(deserialized_value)?;
        Ok(())
    }

    fn init(&mut self) {
        // self.client.initialize();
    }

    fn file(&self) -> &PathBuf {
        self.storage.file()
    }
}

#[cfg(test)]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MyStruct {
    #[serde(rename = "myNumber")]
    pub my_int: i32,
    pub my_bool: bool,
    #[serde(default)]
    pub my_nullable_enum: Option<MyEnum>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
enum MyEnum {
    StringNewType(String),
    StructVariant { floats: Vec<f32> },
}

mod tests {
    #[test]
    fn internal() {
        assert_eq!(1, 1);
    }
}
