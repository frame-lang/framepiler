use std::convert::TryFrom;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub enum TargetLanguage {
    Python3,
    TypeScript,
    Graphviz,
    C,
    Cpp,
    Java,
    CSharp,
    Rust,
    Go,
    JavaScript,
    Php,
    Kotlin,
    Swift,
    Ruby,
    Erlang,
    Lua,
    Dart,
    GDScript,
}

impl TargetLanguage {
    pub fn file_extension(&self) -> &'static str {
        match self {
            TargetLanguage::Python3 => "py",
            TargetLanguage::TypeScript => "ts",
            TargetLanguage::Graphviz => "graphviz",
            TargetLanguage::C => "c",
            TargetLanguage::Cpp => "cpp",
            TargetLanguage::Java => "java",
            TargetLanguage::CSharp => "cs",
            TargetLanguage::Rust => "rs",
            TargetLanguage::Go => "go",
            TargetLanguage::JavaScript => "js",
            TargetLanguage::Php => "php",
            TargetLanguage::Kotlin => "kt",
            TargetLanguage::Swift => "swift",
            TargetLanguage::Ruby => "rb",
            TargetLanguage::Erlang => "erl",
            TargetLanguage::Lua => "lua",
            TargetLanguage::Dart => "dart",
            TargetLanguage::GDScript => "gd",
        }
    }
}

impl TryFrom<&str> for TargetLanguage {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let normalized = value.to_ascii_lowercase();
        if normalized == "python_3" || normalized == "python" {
            Ok(TargetLanguage::Python3)
        } else if normalized == "typescript" || normalized == "ts" {
            Ok(TargetLanguage::TypeScript)
        } else if normalized == "graphviz" {
            Ok(TargetLanguage::Graphviz)
        } else if normalized == "c" {
            Ok(TargetLanguage::C)
        } else if normalized == "c++" || normalized == "cpp" || normalized == "cpp_17" || normalized == "cpp_20" {
            Ok(TargetLanguage::Cpp)
        } else if normalized == "java" {
            Ok(TargetLanguage::Java)
        } else if normalized == "csharp" || normalized == "c#" || normalized == "cs" {
            Ok(TargetLanguage::CSharp)
        } else if normalized == "rust" || normalized == "rs" {
            Ok(TargetLanguage::Rust)
        } else if normalized == "go" || normalized == "golang" {
            Ok(TargetLanguage::Go)
        } else if normalized == "javascript" || normalized == "js" {
            Ok(TargetLanguage::JavaScript)
        } else if normalized == "php" {
            Ok(TargetLanguage::Php)
        } else if normalized == "kotlin" || normalized == "kt" {
            Ok(TargetLanguage::Kotlin)
        } else if normalized == "swift" {
            Ok(TargetLanguage::Swift)
        } else if normalized == "ruby" || normalized == "rb" {
            Ok(TargetLanguage::Ruby)
        } else if normalized == "erlang" || normalized == "erl" {
            Ok(TargetLanguage::Erlang)
        } else if normalized == "lua" {
            Ok(TargetLanguage::Lua)
        } else if normalized == "dart" {
            Ok(TargetLanguage::Dart)
        } else if normalized == "gdscript" || normalized == "gd" {
            Ok(TargetLanguage::GDScript)
        } else {
            Err(format!(
                "Unrecognized target language: {}. Supported languages are: python_3, typescript, javascript, rust, c, cpp, java, csharp, go, php, kotlin, swift, ruby, erlang, lua, dart, gdscript, graphviz",
                normalized
            ))
        }
    }
}

impl TryFrom<String> for TargetLanguage {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

