use std::fmt::{Debug, Display, Formatter, Write};
use std::path::{Path, PathBuf};
use std::{env, io};
use std::sync::LazyLock;
use anyhow::ensure;
use artifact_dependency::{CrateType, Profile};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::try_join;

macro_rules! plaintext_sources {
    () => {
        include!("includes/plaintext_sources")
    };
}

macro_rules! json_sources {
    () => {
        include!("includes/json_sources")
    };
}

macro_rules! get_var {
    ($lit: literal) => {{
        println!("cargo::rerun-if-env-changed={}", $lit);
        env::var($lit).map_err(|e| io::Error::other(format!(concat!($lit, " {err}"), err = e)))
    }};
}

static OUT_DIR: LazyLock<&Path> = LazyLock::new(|| {
    let boxed = PathBuf::from(get_var!("OUT_DIR").unwrap())
        .into_boxed_path();
    
    Box::leak(boxed)
});

async fn make_default_sources_toml() -> anyhow::Result<()> {
    let mut data = String::new();

    let plain_sources = plaintext_sources!();
    for source in plain_sources {
        writeln!(data, r#"["{source}"]"#)?;
        writeln!(data, "steps = [\"Plaintext\"]\n")?;
    }

    let plain_sources = json_sources!();
    for (source, key) in plain_sources {
        writeln!(data, r#"["{source}"]"#)?;
        writeln!(data, r#"steps = [{{ Json = {{ key = "{key}" }} }}]"#)?;
    }

    tokio::fs::write(OUT_DIR.join("sources.toml"), data.trim()).await?;
    Ok(())
}

async fn make_default_sources_rs() -> anyhow::Result<()> {
    let mut file = BufWriter::new(File::create(OUT_DIR.join("sources.array")).await?);

    #[derive(Clone)]
    struct VecDebug<T>(Vec<T>);

    impl<T: Debug> Debug for VecDebug<T> {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.write_str("vec!")?;
            <[T] as Debug>::fmt(&self.0, f)
        }
    }

    #[derive(Clone)]
    struct DisplayStr(String);

    impl Debug for DisplayStr {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            <str as Display>::fmt(&self.0, f)
        }
    }

    macro_rules! vec {
        [$($args:tt)*] => {
            VecDebug(::std::vec![$($args)*])
        };
    }

    macro_rules! format {
        ($($args:tt)*) => {
            DisplayStr(::std::format!($($args)*))
        };
    }

    let mut sources = plaintext_sources!().map(|url| (url, vec![])).to_vec();

    sources.extend(json_sources!().map(|(source, key)| {
        (
            source,
            vec![format!(
                r#"ProcessStep::Json {{ key: "{}".into() }}"#,
                key.escape_debug()
            )],
        )
    }));

    file.write_all(format!("{sources:?}").0.as_bytes()).await?;

    file.flush().await?;

    Ok(())
}

async fn generate_dispatcher() -> anyhow::Result<()> {


    if get_var!("CARGO_CFG_TARGET_OS")? == "linux" {
        println!("cargo::rerun-if-changed=/linux_dispatcher");
        println!("cargo::rerun-if-changed=src/network_listener/linux/dispatcher");
    
        let target = get_var!("TARGET")?;

        tokio::task::spawn_blocking(move || {
            let artifact = artifact_dependency::ArtifactDependency::builder()
                .crate_name("linux_dispatcher")
                .artifact_type(CrateType::Executable)
                .profile(Profile::Other("linux-dispatcher".into()))
                .target_name(target.trim())
                .build_always(true)
                .build_missing(false)
                .build()
                .build()
                .map_err(io::Error::other)?;
            
            assert_eq!(artifact.package.authors, ["IS", "LINUX", "DISPATCH", "ARTIFACT"]);
            
            std::fs::copy(
                artifact.path,
                OUT_DIR.join("./dispatcher-bin"),
            ).unwrap();
            
            Ok::<_, io::Error>(())
        }).await??;

        
    }

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    ensure!(
        std::fs::metadata("./includes")?.is_dir(),
        "no includes directory"
    );
    
    println!("cargo::rerun-if-changed=default");
    try_join!(
        make_default_sources_toml(),
        make_default_sources_rs(),
        generate_dispatcher()
    )?;
    
    Ok(())
}
