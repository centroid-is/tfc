use std::{error::Error, path::PathBuf};

struct FileStorage<T> {
    value: T,
    filename: PathBuf,
}

impl<T> FileStorage<T> {
    fn new(&PathBuf path) {
        
    }
    
    fn file(&self) -> &PathBuf {
        filename
    }
    
    fn set_value(mut &self) -> Result<(), Box<dyn Error>> {
        Ok()
    }
}


struct Confman<T> {
    storage: FileStorage<T>,
    key: String,
}

impl<T> Confman<T> {
    fn new(bus: zbus::connection, key: &str) -> Self {
        key = key.clone();
    }

    fn value(&self) {
        self.storage.value()
    }

    fn to_json_string(&self) -> Result<String, Box<dyn Error>> {
        serde::serde_json::to_string(self.value()).map_err(|e| {
            log!(target: key, Level::Warn,  "Error serializing to JSON: {}", e);
            Box::new(e) as Box<dyn Error>
        })
    }

    fn schema(&self) -> String {
        schemars::schema_for!(T)
    }

    fn set_changed(&self) -> Result<(), Box<dyn Error>> {
        json_str = self.to_json_string();
        if Ok(json_str) {
            // self.client.set(json_str);
            // self.storage.set_changed()
            Ok(())
        } else {
            Err(json_str)
        }
    }

    fn from_string(&mut self, value: &str) -> Result<(), Box<dyn Error>> {
        let deserialized_value: T = serde::serde_json::from_str(value)?;
        FileStorage::<T>::set_value(deserialized_value)
        // self.storage.set_value(deserialized_value);
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

// use schemars::{schema_for, JsonSchema};
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