cargo build --release
if ($env:OS -eq "Windows_NT") {
  cp ./target/release/excel_tool.exe ./#excel-tool.exe
}
else {
  cp ./target/release/excel_tool ./#excel-tool
  
  cargo zigbuild --target x86_64-pc-windows-gnullvm --release
  
  cp ./target/x86_64-pc-windows-gnullvm/release/excel_tool.exe ./#excel-tool.exe
}