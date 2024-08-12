use core::panic;
use log::{log, Level};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json;
use std::{
    error::Error,
    fs,
    io::{Read, Write},
    path::PathBuf,
};

use crate::progbase;

struct FileStorage<T> {
    value: T,
    filename: PathBuf,
}

impl<T: for<'de> Deserialize<'de> + Serialize> FileStorage<T> {
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
        let deserialized_value: T = serde_json::from_str(&file_content).unwrap_or_else(|e| {
            panic!(
                "Error: {} Failed to parse file contents: {}",
                e, &file_content
            )
        });

        FileStorage {
            value: deserialized_value,
            filename: path.clone(),
        }
    }

    fn to_json(&self) -> Result<String, Box<dyn Error>> {
        serde_json::to_string(&self.value).map_err(|e| {
            log!(target: &self.filename.to_str().unwrap(), Level::Warn,  "Error serializing to JSON: {}", e);
            Box::new(e) as Box<dyn Error>
        })
    }

    pub fn file(&self) -> &PathBuf {
        &self.filename
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    fn set_changed(self) -> Result<(), Box<dyn Error>> {
        let mut file = std::fs::File::open(self.file().as_path())?;
        file.write_all(self.to_json()?.as_bytes())
            .map_err(|e| Box::new(e) as Box<dyn Error>)
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
