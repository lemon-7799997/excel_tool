use calamine::{open_workbook_auto, DataType, Reader};
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub struct SheetContent {
    pub name: String,
    pub rows: Vec<Vec<calamine::Data>>,
}

pub fn data_vec_to_string_vec(data_vec: &Vec<&calamine::Data>) -> Vec<String> {
    data_vec.iter().map(|d| d.to_string()).collect()
}

pub fn data_vec2_to_string_vec2(data_vec: &Vec<Vec<calamine::Data>>) -> Vec<Vec<String>> {
    data_vec.clone().into_iter().map(|row| row.into_iter().map(|c| c.to_string()).collect::<Vec<_>>()).collect()
}

pub fn num_to_col(mut n: usize) -> String {
    let mut col = String::new();
    while n > 0 {
        n -= 1; // Excel 列是 1-based
        let ch = ((n % 26) as u8 + b'A') as char;
        col.insert(0, ch);
        n /= 26;
    }
    col
}

pub fn read_cells(file_path: &Path) -> Result<Vec<SheetContent>, Box<dyn Error>> {
    let mut workbook = open_workbook_auto(file_path).expect("Cannot Open Excel File");
    let mut result: Vec<SheetContent> = Vec::new();
    for sheet in workbook.sheet_names().to_owned() {
        let range = workbook.worksheet_range(&sheet)?;
        let sheet_name = sheet.to_string();
        result.push(SheetContent {
            name: sheet_name,
            rows: range.rows().map(|r| r.to_vec()).collect(),
        });
    }
    Ok(result)
}
