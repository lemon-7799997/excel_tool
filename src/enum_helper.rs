use std::collections::HashMap;

pub fn parse_enum_type(s: &str) -> HashMap<String, i64> {
    let mut result = HashMap::new();
    let mut next_value = 0;

    for line in s.lines() {
        let line = line.trim().trim_end_matches(',').trim();

        if line.is_empty() {
            continue;
        }

        if let Some((name, value)) = line.split_once('=') {
            let name = name.trim();
            let value = value.trim().parse::<i64>().unwrap();

            result.insert(name.to_string(), value);
            next_value = value + 1;
        } else {
            result.insert(line.to_string(), next_value);
            next_value += 1;
        }
    }

    result
}
