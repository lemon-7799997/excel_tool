#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]

mod cell_helper;
mod config;
mod custom_encode;
mod enum_helper;
mod excel_helper;
mod flatten;
mod generate;
mod hash;
mod path_helper;
mod refer;
mod vec_helper;

use clap::{Parser, Subcommand};
use config::ConfigArgs;

#[derive(Parser)]
#[command(name = "excel-tool", version, about = "Excel 工具: 默认生成功能 + refer 引用 subcommand")]
struct Cli {
    /// 工具配置文件路径 (默认 #excel-tool.settings.toml, 相对可执行文件所在目录)
    #[arg(default_value = "#excel-tool.settings.toml")]
    config: String,

    /// 配置项: 显式传入的参数会覆盖 toml 配置文件中的同名配置项
    #[command(flatten, next_help_heading = "配置项 (覆盖 toml 配置)")]
    config_args: ConfigArgs,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 引用: 将源 excel 某 sheet 区域数据写入目标 excel 对应位置
    Refer(refer::ReferArgs),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Refer(args)) => refer::run(args)?,
        None => generate::run(&cli)?,
    }
    Ok(())
}
