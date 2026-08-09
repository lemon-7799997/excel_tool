use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde_json::Value;

pub fn get_encode_map<P: AsRef<Path>>(file_path: P) -> Result<HashMap<String, String>, Box<dyn core::error::Error>> {
    let mut file = File::open(&file_path)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;

    let mut result: HashMap<String, String> = HashMap::new();
    let ext = file_path.as_ref().extension().and_then(|s| s.to_str()).unwrap_or("txt");
    if ext == "json" {
        let json: HashMap<String, Value> = serde_json::from_str(&content)?;
        for (key, value) in json {
            result.insert(key, value.to_string());
        }
    } else if ext == "txt" {
        for line in content.lines() {
            let parts: Vec<_> = line.split_whitespace().collect();
            if let Some(key) = parts.get(0) {
                result.insert(key.to_string(), parts.get(1).map_or("0".to_string(), |s| s.to_string()));
            }
        }
    }
    return Ok(result);
}
