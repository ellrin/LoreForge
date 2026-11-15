//! Convert a .docx the way 開啟 Word… does, without the app:
//! `cargo run -p pc-server --example word_convert -- in.docx [out-folder]`.
#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let docx = std::path::PathBuf::from(args.next().expect("a .docx"));
    let dest = args.next().map(std::path::PathBuf::from).unwrap_or_else(|| pc_server::word::default_dest(&docx));
    match pc_server::word::convert(&docx, &dest, &[]).await {
        Ok(c) => println!("{}", serde_json::to_string_pretty(&c).unwrap()),
        Err(e) => eprintln!("{e}"),
    }
}
