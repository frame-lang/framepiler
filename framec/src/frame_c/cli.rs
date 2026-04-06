use crate::frame_c::driver::{Exe, TargetLanguage};
use crate::frame_c::config::FrameConfig;
use clap::{Arg, Command};
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::path::PathBuf;

pub struct Cli {
    stdin_flag: bool,
    path: Option<PathBuf>,
    language: Option<String>,
    multifile: bool,
    output_dir: Option<PathBuf>,
    debug_output: bool,
    /// Validate-only mode
    validate_only: bool,
    /// Validate (structural) and continue
    validate: bool,
    /// Enable strict/native validation (facade mode)
    validate_native: bool,
    /// Emit debug trailers (errors-json, frame-map, visitor-map, debug-manifest)
    emit_debug: bool,
    command: CliCommand,
}

#[derive(Debug, Clone)]
pub enum CliCommand {
    None,
    Init,
    CompileProject { language: String, dir: PathBuf, output_dir: PathBuf, recursive: bool },
    Compile { language: String, file: PathBuf, format: Option<String> },
    ProjectBuild { language: String, output_dir: PathBuf, recursive: bool },
    FidImport { target: String, input: PathBuf, cache_root: Option<PathBuf> },
}

impl Cli {
    pub fn new() -> Cli {
        let matches = Command::new("framec")
            .version(env!("FRAME_VERSION"))
            .about("Frame language transpiler")
            .subcommand_required(false)
            .arg_required_else_help(false)
            .subcommand_precedence_over_arg(true)
            .subcommand(Command::new("init").about("Initialize a new Frame project with frame.toml").arg(Arg::new("name").help("Project name").value_name("NAME").index(1)))
            .subcommand(
                Command::new("project")
                    .about("Project-level operations (optional; PRT-first)")
                    .subcommand(
                        Command::new("build")
                            .about("Build a Frame project using frame.toml")
                            .arg(Arg::new("language").long("language").short('l').value_name("LANG").required(true))
                            .arg(Arg::new("output-dir").long("output-dir").short('o').value_name("DIR").required(true))
                            .arg(Arg::new("recursive").long("recursive").short('r').action(clap::ArgAction::SetTrue))
                    )
            )
            .subcommand(
                Command::new("fid")
                    .about("FID cache operations (optional; PRT-only Phase A)")
                    .subcommand(
                        Command::new("import")
                            .about("Import a FID JSON file into the cache")
                            .arg(Arg::new("target").long("target").short('t').value_name("TARGET").required(true))
                            .arg(Arg::new("cache-root").long("cache-root").value_name("DIR"))
                            .arg(Arg::new("input").value_name("FID_JSON").required(true))
                    )
            )
            .subcommand(
                Command::new("compile")
                    .about("Compile a Frame source file")
                    .arg(Arg::new("language").long("language").short('l').value_name("LANG").required(true))
                    .arg(Arg::new("format").long("format").short('f').value_name("FORMAT").help("Output format: code (default) or model (JSON)"))
                    .arg(Arg::new("file").value_name("FILE").required(true))
            )
            .subcommand(
                Command::new("compile-project")
                    .about("Compile all Frame source files in a directory")
                    .arg(Arg::new("language").long("language").short('l').value_name("LANG").required(true))
                    .arg(Arg::new("output-dir").long("output-dir").short('o').value_name("DIR").required(true))
                    .arg(Arg::new("recursive").long("recursive").short('r').action(clap::ArgAction::SetTrue))
                    .arg(Arg::new("dir").value_name("DIR").required(true))
            )
            .arg(Arg::new("FILE-PATH").help("File path").value_name("FILE").index(1))
            .arg(Arg::new("language").value_name("LANG").long("language").short('l').help("Target language (python_3, typescript, javascript, rust, c, cpp, java, kotlin, swift, ruby, csharp, go, php, graphviz)").num_args(1))
            .arg(Arg::new("multifile").long("multifile").short('m').help("Enable multi-file project compilation").action(clap::ArgAction::SetTrue))
            .arg(Arg::new("output-dir").long("output-dir").short('o').help("Output directory for generated files (compile/multi-file)").value_name("DIR").num_args(1).global(true))
            .arg(Arg::new("debug-output").long("debug-output").help("Generate JSON output with transpiled code and source map").action(clap::ArgAction::SetTrue).global(true))
            .arg(Arg::new("validate").long("validate").help("Run validation before transpile").action(clap::ArgAction::SetTrue).global(true))
            .arg(Arg::new("validate-syntax").long("validate-syntax").help("Alias for --validate (compat) ").action(clap::ArgAction::SetTrue).global(true))
            .arg(Arg::new("validation-only").long("validation-only").help("Run validation only and exit with status").action(clap::ArgAction::SetTrue).global(true))
            .arg(Arg::new("validation-level").long("validation-level").help("Validation level (compat)").num_args(1).global(true))
            .arg(Arg::new("validate-native").long("validate-native").help("Enable strict/native validation (facade mode)").action(clap::ArgAction::SetTrue).global(true))
            .arg(Arg::new("validation-format").long("validation-format").help("Validation output format (compat)").num_args(1).global(true))
            .arg(Arg::new("emit-debug").long("emit-debug").help("Emit debug trailers: errors-json, frame-map, visitor-map (module), debug-manifest").action(clap::ArgAction::SetTrue).global(true))
            
            .get_matches();

        let mut has_subcommand = false;
        let command = match matches.subcommand() {
            Some((name, sub)) => {
                has_subcommand = true;
                match name {
                    "init" => CliCommand::Init,
                    "compile-project" => {
                        let lang = sub.get_one::<String>("language").unwrap_or_else(|| { eprintln!("error: language required"); std::process::exit(exitcode::USAGE); }).to_string();
                        let dir = sub.get_one::<String>("dir").map(|s| PathBuf::from(s)).unwrap_or_else(|| { eprintln!("error: dir required"); std::process::exit(exitcode::USAGE); });
                        let out = sub.get_one::<String>("output-dir").map(|s| PathBuf::from(s)).unwrap_or_else(|| { eprintln!("error: output-dir required"); std::process::exit(exitcode::USAGE); });
                        let recursive = sub.get_flag("recursive");
                        CliCommand::CompileProject { language: lang, dir, output_dir: out, recursive }
                    }
                    "compile" => {
                        let lang = sub.get_one::<String>("language").unwrap_or_else(|| { eprintln!("error: language required"); std::process::exit(exitcode::USAGE); }).to_string();
                        let file = sub.get_one::<String>("file").map(|s| PathBuf::from(s)).unwrap_or_else(|| { eprintln!("error: file required"); std::process::exit(exitcode::USAGE); });
                        let format = sub.get_one::<String>("format").cloned();
                        CliCommand::Compile { language: lang, file, format }
                    }
                    "project" => {
                        match sub.subcommand() {
                            Some(("build", sb)) => {
                                let lang = sb.get_one::<String>("language").unwrap_or_else(|| { eprintln!("error: language required"); std::process::exit(exitcode::USAGE); }).to_string();
                                let out = sb.get_one::<String>("output-dir").map(|s| PathBuf::from(s)).unwrap_or_else(|| { eprintln!("error: output-dir required"); std::process::exit(exitcode::USAGE); });
                                let recursive = sb.get_flag("recursive");
                                CliCommand::ProjectBuild { language: lang, output_dir: out, recursive }
                            }
                            _ => CliCommand::None,
                        }
                    }
                    "fid" => {
                        match sub.subcommand() {
                            Some(("import", sb)) => {
                                let target = sb.get_one::<String>("target").unwrap_or_else(|| { eprintln!("error: target required"); std::process::exit(exitcode::USAGE); }).to_string();
                                let input = sb.get_one::<String>("input").map(|s| PathBuf::from(s)).unwrap_or_else(|| { eprintln!("error: FID_JSON required"); std::process::exit(exitcode::USAGE); });
                                let cache_root = sb.get_one::<String>("cache-root").map(|s| PathBuf::from(s));
                                CliCommand::FidImport { target, input, cache_root }
                            }
                            _ => CliCommand::None,
                        }
                    }
                    _ => CliCommand::None,
                }
            }
            None => CliCommand::None,
        };

        let mut stdin = false;
        let mut path_opt = None;
        if !has_subcommand && matches.contains_id("FILE-PATH") {
            let file_path = matches.get_one::<String>("FILE-PATH");
            path_opt = file_path.map(|file_path| PathBuf::from(file_path.clone()));
        } else if !has_subcommand {
            stdin = true;
        }

        let language_opt = matches.get_one::<String>("language").map(|s| s.clone());
        let multifile = matches.get_flag("multifile");
        let output_dir_opt = matches.get_one::<String>("output-dir").map(|s| PathBuf::from(s.clone()));
        let debug_output = matches.get_flag("debug-output");
        let validate_only = matches.get_flag("validation-only");
        let validate = matches.get_flag("validate") || matches.get_flag("validate-syntax");
        let validate_native = matches.get_flag("validate-native");
        let emit_debug = matches.get_flag("emit-debug");

        Cli {
            stdin_flag: stdin,
            path: path_opt,
            language: language_opt,
            multifile,
            output_dir: output_dir_opt,
            debug_output,
            validate_only,
            validate,
            validate_native,
            emit_debug,
            command,
        }
    }
}

impl Default for Cli {
    fn default() -> Self {
        Cli::new()
    }
}

pub fn run() {
    run_with(Cli::new());
}

pub fn run_with(args: Cli) {
    match args.command {
        CliCommand::Init => {
            handle_init_command();
            return;
        }
        CliCommand::ProjectBuild { language, ref output_dir, recursive } => {
            // PRT-first, advisory project build:
            // - If a frame.toml is found, use its root and source dirs.
            // - Otherwise, delegate to compile-project over the current directory.
            //
            // Project builds should honour the same validation/debug flags as
            // direct compile-project invocations, so we thread those settings
            // through when we construct the inner Cli value.
            if let Some((config_path, cfg)) = FrameConfig::find_project_config() {
                let project_root = config_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let src_dirs: Vec<std::path::PathBuf> = if !cfg.build.source_dirs.is_empty() {
                    cfg.build
                        .source_dirs
                        .iter()
                        .map(|p| project_root.join(p))
                        .collect()
                } else if !cfg.paths.modules.is_empty() {
                    cfg.paths
                        .modules
                        .iter()
                        .map(|p| project_root.join(p))
                        .collect()
                } else {
                    vec![project_root.join("src")]
                };
                for dir in src_dirs {
                    let project_args = Cli {
                        stdin_flag: false,
                        path: None,
                        // The compile-project command carries the language/dir/output;
                        // we keep the top-level language field unused here.
                        language: None,
                        multifile: false,
                        output_dir: Some(output_dir.clone()),
                        debug_output: args.debug_output,
                        validate_only: args.validate_only,
                        validate: args.validate,
                        validate_native: args.validate_native,
                        emit_debug: args.emit_debug,
                        command: CliCommand::CompileProject {
                            language: language.clone(),
                            dir: dir.clone(),
                            output_dir: output_dir.clone(),
                            recursive,
                        },
                    };
                    run_with(project_args);
                }
            } else {
                let dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let project_args = Cli {
                    stdin_flag: false,
                    path: None,
                    language: None,
                    multifile: false,
                    output_dir: Some(output_dir.clone()),
                    debug_output: args.debug_output,
                    validate_only: args.validate_only,
                    validate: args.validate,
                    validate_native: args.validate_native,
                    emit_debug: args.emit_debug,
                    command: CliCommand::CompileProject {
                        language,
                        dir: dir.clone(),
                        output_dir: output_dir.clone(),
                        recursive,
                    },
                };
                run_with(project_args);
            }
            return;
        }
        CliCommand::FidImport { target, input, cache_root } => {
            // Phase A: simple file copy into the FID cache layout. This does not
            // invoke external tools; it only organizes existing JSON into the
            // expected `.frame/cache/fid/<target>/` directory.
            let target_dir = match target.as_str() {
                "python" | "python_3" => "python",
                "typescript" | "ts" => "typescript",
                "rust" | "rs" => "rust",
                other => {
                    eprintln!(
                        "Unsupported FID target '{}'; expected one of python, python_3, typescript, ts, rust, rs",
                        other
                    );
                    std::process::exit(exitcode::USAGE);
                }
            };
            let root = cache_root.unwrap_or_else(|| std::path::PathBuf::from(".frame/cache/fid"));
            let dest_dir = root.join(target_dir);
            if let Err(e) = std::fs::create_dir_all(&dest_dir) {
                eprintln!("Failed to create FID cache directory {:?}: {}", dest_dir, e);
                std::process::exit(exitcode::IOERR);
            }
            let file_name = input
                .file_name()
                .map(|s| s.to_owned())
                .unwrap_or_else(|| std::ffi::OsStr::new("fid.json").to_owned());
            let dest_path = dest_dir.join(&file_name);
            match std::fs::copy(&input, &dest_path) {
                Ok(_) => {
                    println!("Imported FID: {}", dest_path.display());
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Failed to import FID from {:?} to {:?}: {}", input, dest_path, e);
                    std::process::exit(exitcode::IOERR);
                }
            }
        }
        CliCommand::CompileProject { language, dir, output_dir, recursive } => {
            let lang = match TargetLanguage::try_from(language) { Ok(l) => l, Err(e) => { eprintln!("Invalid target language: {}", e); std::process::exit(exitcode::USAGE); } };
            let allowed_targets: std::collections::HashSet<&str> = match lang {
                TargetLanguage::Python3 => ["python_3", "python"].into_iter().collect(),
                TargetLanguage::TypeScript => ["typescript", "ts"].into_iter().collect(),
                TargetLanguage::Rust => ["rust", "rs"].into_iter().collect(),
                TargetLanguage::CSharp => ["csharp"].into_iter().collect(),
                TargetLanguage::C => ["c"].into_iter().collect(),
                TargetLanguage::Cpp => ["cpp", "c++"].into_iter().collect(),
                TargetLanguage::Java => ["java"].into_iter().collect(),
                TargetLanguage::Go => ["go", "golang"].into_iter().collect(),
                TargetLanguage::JavaScript => ["javascript", "js"].into_iter().collect(),
                TargetLanguage::Php => ["php"].into_iter().collect(),
                TargetLanguage::Kotlin => ["kotlin", "kt"].into_iter().collect(),
                TargetLanguage::Swift => ["swift"].into_iter().collect(),
                TargetLanguage::Ruby => ["ruby", "rb"].into_iter().collect(),
                TargetLanguage::Erlang => ["erlang", "erl"].into_iter().collect(),
                TargetLanguage::Lua => ["lua"].into_iter().collect(),
                TargetLanguage::Dart => ["dart"].into_iter().collect(),
                TargetLanguage::GDScript => ["gdscript"].into_iter().collect(),
                TargetLanguage::Graphviz => ["graphviz"].into_iter().collect(),
            };
            fn detect_target(content: &str) -> Option<String> {
                for line in content.lines() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("@target") {
                        let rest = trimmed["@target".len()..].trim();
                        let token = rest.split_whitespace().next()?;
                        let clean = token.trim_matches(|c| c == '"' || c == '\'').to_lowercase();
                        return Some(clean);
                    }
                }
                None
            }
            // Walk directory, compile module files (@target present), write outputs to output_dir
            fn iter(dir: &std::path::Path, recursive: bool) -> std::io::Result<Vec<std::path::PathBuf>> {
                let mut out = Vec::new();
                fn walk(acc: &mut Vec<std::path::PathBuf>, p: &std::path::Path, recursive: bool) -> std::io::Result<()> {
                    for entry in std::fs::read_dir(p)? {
                        let entry = entry?; let path = entry.path();
                        if path.is_dir() { if recursive { walk(acc, &path, recursive)?; } }
                        else if path.is_file() { acc.push(path); }
                    }
                    Ok(())
                }
                walk(&mut out, dir, recursive)?; Ok(out)
            }
            let files = match iter(&dir, recursive) { Ok(v) => v, Err(e) => { eprintln!("walk error: {}", e); std::process::exit(exitcode::IOERR); } };
            // Respect debug/map flags for trailers
            if args.debug_output { std::env::set_var("FRAME_ERROR_JSON", "1"); }
            if args.emit_debug {
                std::env::set_var("FRAME_ERROR_JSON", "1");
                std::env::set_var("FRAME_MAP_TRAILER", "1");
                std::env::set_var("FRAME_DEBUG_MANIFEST", "1");
            }
            let output_root = output_dir.join("build");
            if let Err(e) = std::fs::create_dir_all(&output_root) { eprintln!("cannot create output dir: {}", e); std::process::exit(exitcode::IOERR); }
            let mut compiled: Vec<String> = Vec::new();
            let mut had_errors = false;
            let mut errors_count: usize = 0;
            let mut validated_count: usize = 0;
            let mut missing_target: Vec<std::path::PathBuf> = Vec::new();
            let mut mismatched_target: Vec<(std::path::PathBuf, String)> = Vec::new();
            let mut dup_systems: BTreeMap<String, Vec<std::path::PathBuf>> = BTreeMap::new();
            for f in files {
                let Ok(content) = std::fs::read_to_string(&f) else { continue };
                let target_decl = match detect_target(&content) {
                    Some(t) => t,
                    None => {
                        missing_target.push(f.clone());
                        had_errors = true;
                        continue;
                    }
                };
                if !allowed_targets.contains(target_decl.as_str()) {
                    mismatched_target.push((f.clone(), target_decl));
                    had_errors = true;
                    continue;
                }
                // Check for duplicate system names across modules (best-effort)
                if let Some(sys_name) = crate::frame_c::compiler::find_system_name(content.as_bytes(), 0) {
                    let entry = dup_systems.entry(sys_name).or_insert_with(Vec::new);
                    entry.push(f.clone());
                }
                if args.validate || args.validate_only {
                    match crate::frame_c::compiler::validate_module_with_mode(&content, lang, args.validate_native) {
                        Ok(res) => {
                            let mut had_any = false;
                            for issue in &res.issues { eprintln!("{}: validation: {}", f.display(), issue.message); had_any = true; }
                            if had_any { had_errors = true; }
                            errors_count += res.issues.len();
                            validated_count += 1;
                            if args.validate_only && !res.ok { /* defer exit to post-loop */ }
                            if args.validate_native && !res.ok { /* continue; we'll still compile but print issues */ }
                        }
                        Err(e) => { eprintln!("{}: validation error: {}", f.display(), e.error); if args.validate_only || args.validate_native { std::process::exit(e.code); } }
                    }
                }
                if args.validate_only { continue; }
                match crate::frame_c::compiler::compile_module(&content, lang) {
                    Ok(code) => {
                        let ext = match lang { TargetLanguage::Python3 => ".py", TargetLanguage::TypeScript => ".ts", TargetLanguage::CSharp => ".cs", TargetLanguage::C => ".c", TargetLanguage::Cpp => ".cpp", TargetLanguage::Java => ".java", TargetLanguage::Rust => ".rs", TargetLanguage::Go => ".go", TargetLanguage::JavaScript => ".js", TargetLanguage::Php => ".php", TargetLanguage::Kotlin => ".kt", TargetLanguage::Swift => ".swift", TargetLanguage::Ruby => ".rb", TargetLanguage::Erlang => ".erl", TargetLanguage::Lua => ".lua", TargetLanguage::Dart => ".dart", TargetLanguage::GDScript => ".gd", TargetLanguage::Graphviz => ".dot" };
                        let _stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
                        let lang_dir = match lang {
                            TargetLanguage::Python3 => "python",
                            TargetLanguage::TypeScript => "typescript",
                            TargetLanguage::JavaScript => "javascript",
                            TargetLanguage::CSharp => "csharp",
                            TargetLanguage::C => "c",
                            TargetLanguage::Cpp => "cpp",
                            TargetLanguage::Java => "java",
                            TargetLanguage::Rust => "rust",
                            TargetLanguage::Go => "go",
                            TargetLanguage::Php => "php",
                            TargetLanguage::Kotlin => "kotlin",
                            TargetLanguage::Swift => "swift",
                            TargetLanguage::Ruby => "ruby",
                            TargetLanguage::Erlang => "erlang",
                            TargetLanguage::Lua => "lua",
                            TargetLanguage::Dart => "dart",
                            TargetLanguage::GDScript => "gdscript",
                            TargetLanguage::Graphviz => "graphviz",
                        };
                        let rel = f.strip_prefix(&dir).unwrap_or(&f);
                        let mut outp = output_root.join(lang_dir).join(rel);
                        outp.set_extension(ext.trim_start_matches('.'));
                        if let Some(parent) = outp.parent() {
                            if let Err(e) = std::fs::create_dir_all(parent) {
                                eprintln!("cannot create output dir: {}", e);
                                std::process::exit(exitcode::IOERR);
                            }
                        }
                        if let Err(e) = std::fs::write(&outp, code) { eprintln!("write error: {}", e); std::process::exit(exitcode::IOERR); }
                        compiled.push(outp.display().to_string());
                    }
                    Err(e) => { eprintln!("{}", e.error); std::process::exit(e.code); }
                }
            }
            if !missing_target.is_empty() {
                for p in &missing_target {
                    eprintln!("{}: missing @target declaration (compile-project requires explicit @target per module)", p.display());
                }
            }
            if !mismatched_target.is_empty() {
                for (p, t) in &mismatched_target {
                    eprintln!("{}: @target '{}' does not match requested project target", p.display(), t);
                }
            }
            for (sys, paths) in &dup_systems {
                if paths.len() > 1 {
                    eprintln!("Duplicate system '{}' across modules:", sys);
                    for p in paths {
                        eprintln!("  - {}", p.display());
                    }
                }
            }
            if args.validate_only {
                println!("[compile-project] summary: validated={} errors={}", validated_count, errors_count);
                // Fail if no modules were validated or if any had errors
                if validated_count == 0 || had_errors { std::process::exit(exitcode::DATAERR); }
                else { std::process::exit(0); }
            }
            if !missing_target.is_empty() || !mismatched_target.is_empty() || dup_systems.values().any(|v| v.len() > 1) {
                std::process::exit(exitcode::DATAERR);
            }
            // Print a simple manifest for now
            println!("Compiled {} module(s)", compiled.len());
            for p in &compiled { println!("{}", p); }
            // For Python projects, copy frame_runtime_py once to the output directory root
            if matches!(lang, TargetLanguage::Python3) {
                if let Some(outdir) = args.output_dir.as_ref() {
                    let outdir = outdir.join("build").join("python");
                    let env_override = std::env::var("FRAME_RUNTIME_PY_DIR").ok().map(std::path::PathBuf::from);
                    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
                    let repo_guess = exe_dir.as_ref()
                        .and_then(|d| d.parent().map(|d| d.to_path_buf()))
                        .and_then(|d| d.parent().map(|d| d.to_path_buf()))
                        .map(|d| d.join("frame_runtime_py"));
                    let target_guess = exe_dir.as_ref()
                        .and_then(|d| d.parent().map(|d| d.to_path_buf()))
                        .map(|d| d.join("frame_runtime_py"));
                    let cwd_guess = Some(std::path::PathBuf::from("frame_runtime_py"));
                    let runtime_src = env_override
                        .filter(|p| p.exists())
                        .or(repo_guess.filter(|p| p.exists()))
                        .or(target_guess.filter(|p| p.exists()))
                        .or(cwd_guess.filter(|p| p.exists()))
                        .unwrap_or_else(|| std::path::PathBuf::from("frame_runtime_py"));
                    let dst_dir = outdir.join("frame_runtime_py");
                    if runtime_src.exists() {
                        fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
                            if !dst.exists() { std::fs::create_dir_all(dst)?; }
                            for entry in std::fs::read_dir(src)? {
                                let entry = entry?; let p = entry.path();
                                let name = entry.file_name(); let to = dst.join(name);
                                if p.is_dir() { copy_dir(&p, &to)?; } else if p.is_file() { std::fs::copy(&p, &to)?; }
                            }
                            Ok(())
                        }
                        if let Err(e) = copy_dir(&runtime_src, &dst_dir) {
                            eprintln!("warning: failed to copy frame_runtime_py: {}", e);
                        }
                    } else {
                        eprintln!("warning: frame_runtime_py not found at {:?}; set FRAME_RUNTIME_PY_DIR to override", runtime_src);
                    }
                }
            }
            // For TypeScript projects, copy frame_runtime_ts once to the output directory root
            if matches!(lang, TargetLanguage::TypeScript) {
                if let Some(outdir) = args.output_dir.as_ref() {
                    let outdir = outdir.join("build").join("typescript");
                    let env_override = std::env::var("FRAME_RUNTIME_TS_DIR").ok().map(std::path::PathBuf::from);
                    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
                    let repo_guess = exe_dir.as_ref()
                        .and_then(|d| d.parent().map(|d| d.to_path_buf()))
                        .and_then(|d| d.parent().map(|d| d.to_path_buf()))
                        .map(|d| d.join("frame_runtime_ts"));
                    let target_guess = exe_dir.as_ref()
                        .and_then(|d| d.parent().map(|d| d.to_path_buf()))
                        .map(|d| d.join("frame_runtime_ts"));
                    let cwd_guess = Some(std::path::PathBuf::from("frame_runtime_ts"));
                    let runtime_src = env_override
                        .filter(|p| p.exists())
                        .or(repo_guess.filter(|p| p.exists()))
                        .or(target_guess.filter(|p| p.exists()))
                        .or(cwd_guess.filter(|p| p.exists()))
                        .unwrap_or_else(|| std::path::PathBuf::from("frame_runtime_ts"));
                    let dst_dir = outdir.join("frame_runtime_ts");
                    if runtime_src.exists() {
                        fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
                            if !dst.exists() { std::fs::create_dir_all(dst)?; }
                            for entry in std::fs::read_dir(src)? {
                                let entry = entry?; let p = entry.path();
                                let name = entry.file_name(); let to = dst.join(name);
                                if p.is_dir() { copy_dir(&p, &to)?; } else if p.is_file() { std::fs::copy(&p, &to)?; }
                            }
                            Ok(())
                        }
                        if let Err(e) = copy_dir(&runtime_src, &dst_dir) {
                            eprintln!("warning: failed to copy frame_runtime_ts: {}", e);
                        }
                    } else {
                        eprintln!("warning: frame_runtime_ts not found at {:?}; set FRAME_RUNTIME_TS_DIR to override", runtime_src);
                    }
                }
            }
            return;
        }
        CliCommand::Compile { language, file, format } => {
            let lang = match TargetLanguage::try_from(language) { Ok(l) => l, Err(e) => { eprintln!("Invalid target language: {}", e); std::process::exit(exitcode::USAGE); } };
            match std::fs::read_to_string(&file) {
                Ok(content) => {
                    // --format model: emit semantic JSON model instead of code
                    if format.as_deref() == Some("model") {
                        let target_lang = crate::frame_c::compiler::TargetLanguage::from(lang);
                        let compiler = crate::frame_c::compiler::FrameCompiler::new(target_lang);
                        match compiler.compile_to_model(&content, file.to_str().unwrap_or("<unknown>"), lang.file_extension()) {
                            Ok(json) => {
                                if let Some(dir) = args.output_dir.as_ref() {
                                    if let Err(e) = std::fs::create_dir_all(dir) { eprintln!("cannot create output dir: {}", e); std::process::exit(exitcode::IOERR); }
                                    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
                                    let out_path = dir.join(format!("{}.json", stem));
                                    if let Err(e) = std::fs::write(&out_path, &json) { eprintln!("write error: {}", e); std::process::exit(exitcode::IOERR); }
                                    println!("{}", out_path.display());
                                } else {
                                    println!("{}", json);
                                }
                            }
                            Err(e) => {
                                eprintln!("{}", e);
                                std::process::exit(exitcode::DATAERR);
                            }
                        }
                        return;
                    }

                    if args.debug_output { std::env::set_var("FRAME_ERROR_JSON", "1"); }
                    if args.emit_debug {
                        std::env::set_var("FRAME_ERROR_JSON", "1");
                        std::env::set_var("FRAME_MAP_TRAILER", "1");
                        std::env::set_var("FRAME_DEBUG_MANIFEST", "1");
                    }
                    // Compile Frame file
                    let target_lang = crate::frame_c::compiler::TargetLanguage::from(lang);
                    let compiler = crate::frame_c::compiler::FrameCompiler::new(target_lang);

                    match compiler.compile(&content, file.to_str().unwrap_or("<unknown>")) {
                        crate::frame_c::compiler::FrameResult::Ok(output) => {
                            let code = output.code;
                            if let Some(dir) = args.output_dir.as_ref() {
                                if let Err(e) = std::fs::create_dir_all(dir) { eprintln!("cannot create output dir: {}", e); std::process::exit(exitcode::IOERR); }
                                let ext = match lang { TargetLanguage::Python3 => ".py", TargetLanguage::TypeScript => ".ts", TargetLanguage::CSharp => ".cs", TargetLanguage::C => ".c", TargetLanguage::Cpp => ".cpp", TargetLanguage::Java => ".java", TargetLanguage::Rust => ".rs", TargetLanguage::Go => ".go", TargetLanguage::JavaScript => ".js", TargetLanguage::Php => ".php", TargetLanguage::Kotlin => ".kt", TargetLanguage::Swift => ".swift", TargetLanguage::Ruby => ".rb", TargetLanguage::Erlang => ".erl", TargetLanguage::Lua => ".lua", TargetLanguage::Dart => ".dart", TargetLanguage::GDScript => ".gd", TargetLanguage::Graphviz => ".dot" };
                                let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
                                let out_path = dir.join(format!("{}{}", stem, ext));
                                if let Err(e) = std::fs::write(&out_path, code) { eprintln!("write error: {}", e); std::process::exit(exitcode::IOERR); }
                                // Emit Python runtime package next to outputs when compiling Python modules
                                if matches!(lang, TargetLanguage::Python3) {
                                    // Resolve runtime source directory robustly for compile -o
                                    let env_override = std::env::var("FRAME_RUNTIME_PY_DIR").ok().map(std::path::PathBuf::from);
                                    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
                                    let repo_guess = exe_dir.as_ref()
                                        .and_then(|d| d.parent().map(|d| d.to_path_buf()))
                                        .and_then(|d| d.parent().map(|d| d.to_path_buf()))
                                        .map(|d| d.join("frame_runtime_py"));
                                    let target_guess = exe_dir.as_ref()
                                        .and_then(|d| d.parent().map(|d| d.to_path_buf()))
                                        .map(|d| d.join("frame_runtime_py"));
                                    let cwd_guess = Some(std::path::PathBuf::from("frame_runtime_py"));
                                    let runtime_src = env_override
                                        .filter(|p| p.exists())
                                        .or(repo_guess.filter(|p| p.exists()))
                                        .or(target_guess.filter(|p| p.exists()))
                                        .or(cwd_guess.filter(|p| p.exists()))
                                        .unwrap_or_else(|| std::path::PathBuf::from("frame_runtime_py"));
                                    let dst_dir = dir.join("frame_runtime_py");
                                    if runtime_src.exists() {
                                        // Recursively copy (create dirs as needed)
                                        fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
                                            if !dst.exists() { std::fs::create_dir_all(dst)?; }
                                            for entry in std::fs::read_dir(src)? {
                                                let entry = entry?; let p = entry.path();
                                                let name = entry.file_name(); let to = dst.join(name);
                                                if p.is_dir() {
                                                    copy_dir(&p, &to)?;
                                                } else if p.is_file() {
                                                    std::fs::copy(&p, &to)?; // overwrite if exists
                                                }
                                            }
                                            Ok(())
                                        }
                                        if let Err(e) = copy_dir(&runtime_src, &dst_dir) { eprintln!("warning: failed to copy frame_runtime_py: {}", e); }
                                    } else {
                                        eprintln!("warning: frame_runtime_py not found at {:?}; set FRAME_RUNTIME_PY_DIR to override", runtime_src);
                                    }
                                }
                                println!("{}", out_path.display());
                            } else {
                                println!("{}", code);
                            }
                        }
                        crate::frame_c::compiler::FrameResult::Err(err) => {
                            // compilation errors
                            eprintln!("Frame compilation error");
                            for error in err.errors() {
                                eprintln!("{}", error);
                            }
                            std::process::exit(exitcode::DATAERR);
                        }
                    }
                }
                Err(e) => { eprintln!("Failed to read {}: {}", file.display(), e); std::process::exit(exitcode::NOINPUT); }
            }
            return;
        }
        
        CliCommand::None => {}
    }

    let exe = Exe::new();
    // Validation-only pathway
    let target_language = match &args.language {
        Some(lang_str) => match TargetLanguage::try_from(lang_str.clone()) {
            Ok(lang) => Some(lang),
            Err(err) => {
                eprintln!("Invalid target language: {}", err);
                std::process::exit(exitcode::USAGE);
            }
        },
        None => None,
    };
    if args.validate_only || args.validate {
        let path = args.path.clone().unwrap_or_else(|| { eprintln!("error: file path required"); std::process::exit(exitcode::USAGE); });
        if let Ok(content) = std::fs::read_to_string(&path) {
            // Module file validation (@target present)
            let is_module = content.contains("@target ");
            if is_module {
                // Require target language
                let lang = target_language.unwrap_or(TargetLanguage::Python3);
                match super::compiler::validate_module_with_mode(&content, lang, args.validate_native) {
                    Ok(res) => {
                        for issue in res.issues { eprintln!("validation: {}", issue.message); }
                        if args.validate_only { std::process::exit(if res.ok { 0 } else { exitcode::DATAERR }); }
                    }
                    Err(e) => {
                        eprintln!("validation error: {}", e.error);
                        if args.validate_only { std::process::exit(e.code); }
                    }
                }
            } else {
                eprintln!("validation error: Frame files must specify @@target language.");
                if args.validate_only { std::process::exit(exitcode::DATAERR); }
            }
        }
    }

    if args.stdin_flag {
        match exe.run_stdin(target_language) {
            Ok(code) => println!("{}", code),
            Err(err) => {
                eprintln!("{}", err.error);
                std::process::exit(err.code);
            }
        }
        } else {
            let path = match args.path {
                Some(p) => p,
                None => { eprintln!("error: no input file specified"); std::process::exit(exitcode::USAGE); }
            };
            let result = if args.debug_output {
                exe.run_file_debug(&path, target_language)
            } else if args.multifile {
                exe.run_multifile(&path, target_language, args.output_dir)
            } else {
                if args.emit_debug {
                    std::env::set_var("FRAME_ERROR_JSON", "1");
                    std::env::set_var("FRAME_MAP_TRAILER", "1");
                    std::env::set_var("FRAME_DEBUG_MANIFEST", "1");
                }
                exe.run_file(&path, target_language)
            };

        match result {
            Ok(code) => println!("{}", code),
            Err(err) => {
                eprintln!("{}", err.error);
                std::process::exit(err.code);
            }
        }
    }
}

fn handle_init_command() {
    use std::env;
    use std::fs;

    let current_dir = env::current_dir().unwrap_or_else(|_| { eprintln!("error: Failed to get current directory"); std::process::exit(exitcode::USAGE); });
    let config_path = current_dir.join("frame.toml");

    if config_path.exists() {
        eprintln!("frame.toml already exists in this directory");
        std::process::exit(exitcode::CANTCREAT);
    }

    let project_name = current_dir.file_name().and_then(|n| n.to_str()).map(|s| s.to_string());

    match FrameConfig::create_default(&config_path, project_name.as_deref()) {
        Ok(_) => {
            println!("Created frame.toml");
            let src_dir = current_dir.join("src");
            if !src_dir.exists() {
                fs::create_dir(&src_dir).unwrap_or_else(|_| { eprintln!("error: Failed to create src directory"); std::process::exit(exitcode::USAGE); });
                println!("Created src/");
                let main_file = src_dir.join("main.frm");
                let main_content = r#"# Main entry point for Frame project

fn main() {
    print("Hello from Frame!")
}
"#;
                fs::write(&main_file, main_content).unwrap_or_else(|_| { eprintln!("error: Failed to create main.frm"); std::process::exit(exitcode::USAGE); });
                println!("Created src/main.frm");
            }
            println!("\nFrame project initialized successfully!");
        }
        Err(e) => {
            eprintln!("Failed to create frame.toml: {}", e);
            std::process::exit(exitcode::IOERR);
        }
    }
}
