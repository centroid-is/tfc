use core::panic;
use log::{log, Level};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json;
use std::{
    error::Error,
    fs,
    io::{Read, Write},
    marker::PhantomData,
    path::PathBuf,
};

use crate::progbase;

pub struct ConfMan<T> {
    storage: FileStorage<T>,
    key: String,
}

impl<T: Serialize + for<'de> Deserialize<'de> + JsonSchema + Default> ConfMan<T> {
    pub fn new(
        // bus: zbus::Connection,
        key: &str,
    ) -> Self {
        ConfMan {
            storage: FileStorage::<T>::new(&progbase::make_config_file_name(key, "json")),
            key: key.to_string(),
        }
    }

    pub fn value(&self) -> &T {
        &self.storage.value
    }

    pub fn make_change(&mut self) -> Change<Self, T> {
        Change::new(self)
    }

    pub fn to_json(&self) -> Result<String, Box<dyn Error>> {
        serde_json::to_string(self.value()).map_err(|e| {
            log!(target: &self.key, Level::Warn,  "Error serializing to JSON: {}", e);
            Box::new(e) as Box<dyn Error>
        })
    }

    pub fn from_json(&mut self, value: &str) -> Result<(), Box<dyn Error>> {
        let deserialized_value: T = serde_json::from_str(value)?;
        *self.storage.make_change().value_mut() = deserialized_value;
        Ok(())
    }

    pub fn schema(&self) -> String {
        serde_json::to_string_pretty(&schemars::schema_for!(T)).unwrap()
    }

    pub fn file(&self) -> &PathBuf {
        self.storage.file()
    }
}

impl<T: for<'de> Deserialize<'de> + Serialize + Default> ChangeTrait<T> for ConfMan<T> {
    fn set_changed(&mut self) -> Result<(), Box<dyn Error>> {
        self.storage.set_changed()
    }
    fn value_mut(&mut self) -> &mut T {
        self.storage.value_mut()
    }
    fn key(&self) -> &str {
        &self.key
    }
}

trait ChangeTrait<T> {
    fn set_changed(&mut self) -> Result<(), Box<dyn Error>>;
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

struct FileStorage<T> {
    value: T,
    filename: PathBuf,
}

impl<T: for<'de> Deserialize<'de> + Serialize + Default> FileStorage<T> {
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
        }
    }

    fn to_json(&self) -> Result<String, Box<dyn Error>> {
        serde_json::to_string(&self.value).map_err(|e| {
            log!(target: &self.filename.to_str().unwrap(), Level::Warn,  "Error serializing to JSON: {}", e);
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

impl<T: for<'de> Deserialize<'de> + Serialize + Default> ChangeTrait<T> for FileStorage<T> {
    fn set_changed(&mut self) -> Result<(), Box<dyn Error>> {
        self.save_to_file()
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

    #[test]
    fn struct_test() {
        let current_dir = std::env::current_dir().expect("Failed to get current directory");
        let current_dir_str = current_dir
            .to_str()
            .expect("Failed to convert path to string");
        std::env::set_var("CONFIGURATION_DIRECTORY", current_dir_str);
        let config: ConfMan<MyStruct> = ConfMan::new("key");
        assert_eq!(config.value().my_int, 5);
    }
}
