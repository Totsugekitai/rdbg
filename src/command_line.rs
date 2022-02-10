use clap::Parser;

#[derive(Debug, Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// target file
    pub file: String,

    /// arguments passed target file
    #[clap(short, long)]
    pub args: Vec<String>,
}
