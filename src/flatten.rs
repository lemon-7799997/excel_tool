use std::collections::HashMap;
use std::default;

use serde_json::ser::Compound::Map;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct FlattenPart {
    pub name: String,
    pub index: Option<i32>,
}

impl FlattenPart {
    pub fn to_display_string(&self) -> String {
        if let Some(index) = self.index {
            if index >= 0 {
                format!("[{index}]")
            } else {
                "[]".to_string()
            }
        } else {
            self.name.to_string()
        }
    }

    pub fn set_index(&mut self, value: i32) {
        self.index = Some(value);
    }

    pub fn is_index(&self) -> bool {
        self.index.is_some()
    }

    pub fn is_auto_index(&self) -> bool {
        if let Some(index) = self.index {
            index < 0
        } else {
            false
        }
    }

    pub fn new_auto_index() -> Self {
        FlattenPart {
            name: String::from(""),
            index: Some(-1),
        }
    }

    pub fn new(name: String, index: Option<i32>) -> Self {
        Self { name, index }
    }

    pub fn new_index(index: i32) -> Self {
        Self {
            name: index.to_string(),
            index: Some(index),
        }
    }

    pub fn new_name(name: String) -> Self {
        Self { name, index: None }
    }
}

pub fn get_display_string(parts: &[FlattenPart]) -> String {
    let mut result = String::with_capacity(20);
    let mut last_part: Option<&FlattenPart> = None;
    for part in parts {
        if let Some(last) = last_part {
            result.push_str(if last.is_index() && !part.is_index() { "." } else { "" });
        }
        result.push_str(&if let Some(idx) = part.index { format!("[{}]", idx) } else { part.name.to_string() });
        last_part = Some(&part);
    }
    result
}

#[derive(Default, Debug, Clone)]
pub struct FlattenKey {
    pub raw: String,
    pub parts: Vec<FlattenPart>,
}

impl FlattenKey {
    pub fn to_display_string(&self) -> String {
        get_display_string(&self.parts)
    }

    pub fn last_part_string(&self) -> String {
        self.parts.last().map_or("".to_string(), |p| p.name.clone())
    }

    pub fn new<F>(raw: String, case_converter: F) -> Self
    where
        F: Fn(&str) -> String,
    {
        if raw.len() <= 0 {
            return Self::default();
        }

        let mut input = raw.as_str();
        let mut parts: Vec<FlattenPart> = Vec::new();

        while input.len() > 0 {
            if input.starts_with('[') {
                let Some(close_index) = input.find(']') else {
                    return Self::default();
                };
                if close_index < 2 {
                    parts.push(FlattenPart::new_auto_index());
                } else {
                    let num_slice = &input[1..close_index];
                    let mut num_value: i32 = 0;
                    for ch in num_slice.bytes() {
                        let ch_int = (ch - b'0') as i32;
                        if ch_int >= 0 && ch_int <= 9 {
                            num_value = num_value * 10 + ch_int;
                        } else {
                            num_value = -1;
                            break;
                        }
                    }
                    parts.push(FlattenPart::new_index(num_value));
                }
                input = &input[(close_index + 1)..];
            } else {
                let segment_end = input.find('.').unwrap_or(input.len());
                let name_end = match input.find('[') {
                    Some(index) if index < segment_end => index,
                    _ => segment_end,
                };
                parts.push(FlattenPart::new_name(case_converter(&input[..name_end])));
                input = &input[name_end..];
            }

            if input.len() > 0 && input.starts_with('.') {
                input = &input[1..];
            }
        }

        return Self { raw, parts };
    }
}

pub fn set_json_value(json_data: &mut Value, paths: &Vec<FlattenPart>, value: Value) {
    let mut current = json_data;

    for i in 0..paths.len() {
        let curr_part = &paths[i];
        let next_part = paths.get(i + 1);
        // 如果当前路径part是索引(数组)
        if let Some(curr_part_index) = curr_part.index {
            // 如果当前从json路径中取值取到了数组
            if let Value::Array(ref mut curr_arr) = current {
                let curr_arr_len = curr_arr.len();
                let target_index: usize = if curr_part_index >= 0 { curr_part_index as usize } else { curr_arr.len() };
                for j in curr_arr_len..(target_index + 1) {
                    curr_arr.push(Value::Null);
                }
                current = &mut curr_arr[target_index];
            } else {
                // 如果取到的不是数组, 则置空, 由next part判断
                *current = Value::Null;
            }
        } else {
            if let Value::Object(ref mut map) = current {
                current = map.entry(curr_part.name.to_string()).or_insert(Value::Null);
            } else {
                *current = Value::Null;
            }
        }

        if let Some(next_part) = next_part {
            match current {
                Value::Null => {
                    if next_part.index.is_some() {
                        *current = Value::Array(vec![]);
                    } else {
                        *current = Value::Object(serde_json::Map::new());
                    }
                }
                _ => {}
            }
        }
    }
    *current = value;
}

pub fn get_json_list_by_table(head: Vec<&FlattenKey>, tbody: Vec<&Vec<Value>>) -> Vec<Value> {
    let mut result: Vec<Value> = vec![];
    for (i, row) in tbody.iter().enumerate() {
        let mut json_obj: Value = Value::Object(serde_json::Map::new());
        for (j, cell) in row.iter().enumerate() {
            let Some(head) = head.get(j) else {
                continue;
            };
            set_json_value(&mut json_obj, &head.parts, cell.clone());
        }
        result.push(json_obj);
    }
    return result;
}

pub fn fill_auto_indexes(flat_key: &mut FlattenKey, auto_index_map: &mut HashMap<String, usize>) -> usize {
    if flat_key.raw.is_empty() {
        return 0;
    }

    let Some(which_part_is_auto_index) = flat_key.parts.iter().rposition(|p| p.is_auto_index()) else {
        return 0;
    };
    for part_i in 0..flat_key.parts.len() {
        if !flat_key.parts[part_i].is_auto_index() {
            continue;
        }
        if part_i != which_part_is_auto_index {
            flat_key.parts[part_i].set_index(0);
            continue;
        }
        let temp_val = auto_index_map.entry(flat_key.raw.to_string()).or_insert(0);
        flat_key.parts[part_i].set_index(*temp_val as i32);
        *temp_val += 1;
    }
    flat_key.parts.iter().filter(|p| p.is_auto_index()).count()
}
