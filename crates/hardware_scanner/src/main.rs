use hardware_info::{Hardwarescanner, model::ScanResult};

fn main() -> ScanResult {
    let mut gpu_error = None;
    let mut scanner = Hardwarescanner::new()?;
    let info = scanner.initinalize(&mut gpu_error);
    println!("{:#?}\n\n", info.processes());
    println!(
        "{:#?}\n\n",
        info.processes()
            .iter()
            .filter(|p| p.gpu_memory.is_some())
            .collect::<Vec<_>>()
    );
    for error in info.errors() {
        println!("{error}")
    }
    Ok(())
}
