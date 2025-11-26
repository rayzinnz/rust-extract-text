use crc_fast::{checksum_file, CrcAlgorithm::Crc64Nvme};
use extract_text::*;
use helper_lib::*;
use log::*;
use simplelog::*;
use std::{
	error::Error,
	env,
	path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
	thread,
};

fn main()  -> Result<(), Box<dyn Error>> {
    let logger_config = ConfigBuilder::new()
		.set_time_offset_to_local().expect("Failed to get local time offset")
		.set_time_format_custom(format_description!("[hour]:[minute]:[second].[subsecond digits:3]"))
        .build();
	CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Trace, logger_config, TerminalMode::Mixed, ColorChoice::Auto),
			// TermLogger::new(LevelFilter::Debug, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
            // WriteLogger::new(LevelFilter::Info, Config::default(), File::create("my_rust_binary.log").unwrap()),
        ]
    ).unwrap();

	// error!("Bright red error");
    // info!("This only appears in the log file");
    // debug!("This level is currently not enabled for any logger");

    let keep_going = Arc::new(AtomicBool::new(true));
    let keep_going_flag = keep_going.clone();
    let _signaler_handle = thread::spawn(move || {watch_for_quit(keep_going_flag);});

	info!("Current OS: {}", env::consts::OS); //  "macos", "windows", or "linux".
	// let tempfiles_loc = tempfiles_location();

	let starting_dir: String;
	if cfg!(target_os = "windows") {
		starting_dir = String::from(r#"C:\Users\hrag\Sync\Programming\python\FileSearcher\test"#);
	} else if cfg!(target_os = "linux") {
		starting_dir = String::from("/home/ray/MEGA/Rays/Programming/python/file/test_text_extract");
	} else {
		panic!("Unsupported OS");
	}
	let starting_path: PathBuf = PathBuf::from(starting_dir);

    info!("Starting to traverse directory: {:?}", starting_path);
    
    // Walk through all files and directories recursively
    // for entry in WalkDir::new(starting_path)
    //     .into_iter()
    //     .filter_map(|e| e.ok()) // Skip errors
    // {
    //     let path = entry.path();
        
    //     // Process only files (not directories)
    //     if path.is_file() && path.metadata()?.len() < MAX_FILE_SIZE {
	// 		//println!("path: {:?}", path);
	// 		extract_text_from_file(path)?;
    //     }
    // }

	// let path = Path::new(r#"C:\Users\hrag\Sync\Programming\python\file\test_text_extract\txt\text_cp1252.txt"#);
	let path = Path::new(r#"C:\Users\hrag\Sync\Programming\python\file\test_text_extract\binary\main.exe.bin"#);
	// let path = Path::new(r#"C:\Users\hrag\Sync\Programming\python\file\test_text_extract\archives\202010.zip"#);
	// let path = Path::new(r#"C:\Users\hrag\Sync\Programming\python\file\test_text_extract\archives\with_alternative_password.7z"#);
	// let path = Path::new(r#"C:\Users\hrag\Sync\Programming\python\file\test_text_extract\docs\fiche d'evaluation du stagiaire - Loïc Vital.pdf"#);
	// let path = Path::new(r#"c:\Users\hrag\Sync\Programming\python\file\test_text_extract\docs\sample2.pdf"#);
	// let path = Path::new(r#"c:\Users\hrag\Sync\Programming\python\file\test_text_extract\docs\eLIMS-FGS Incident Record Model Template.docm"#);
	// let path = Path::new(r#"c:\Users\hrag\Sync\Programming\python\file\test_text_extract\emails\msg_in_msg_in_msg.msg"#);
	// let path = Path::new(r#""#);

	// let path = Path::new("/home/ray/MEGA/Rays/Programming/python/file/test_text_extract/txt/text_utf8bom.txt");
	// let path = Path::new("/home/ray/MEGA/Rays/Programming/python/file/test_text_extract/docs/sample2.pdf");
	// let path = Path::new("/home/ray/MEGA/Rays/Programming/python/file/test_text_extract/docs/Heinz Watties Reporting V06.xlsb");
	// let path = Path::new("/home/ray/MEGA/Rays/Programming/python/file/test_text_extract/docs/eLIMS-FGS Incident Record Model Template.docm");
	// let path = Path::new("/home/ray/MEGA/Rays/Programming/python/file/test_text_extract/docs/Cover Letter - Rocket Lab - Software Engineer.odt");
	// let path = Path::new("/home/ray/MEGA/Rays/Programming/python/file/test_text_extract/emails/Agworld soil sampling information - Eurofins NZ.msg");
	// let path = Path::new("/home/ray/MEGA/Rays/Programming/python/file/test_text_extract/emails/msg_in_msg_in_msg.msg");
	// let path = Path::new("");

	let file_crc = checksum_file(Crc64Nvme, path.to_str().unwrap(), None).unwrap() as i64;
	debug!("file_crc: {}", file_crc);
	let pre_scanned_items: Vec<FileListItem> = Vec::new();
	let keep_going_flag = keep_going.clone();
	let contents = extract_text_from_file(path, pre_scanned_items, keep_going_flag)?;

	debug!("{:#?}", contents);
    
    info!("Finished traversing directory");
    
    Ok(())
}
