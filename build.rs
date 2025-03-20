use std::fmt::{Debug, Display, Formatter, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::{env, io};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::process::Command;
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

    tokio::fs::write("includes/sources.toml", data.trim()).await?;
    Ok(())
}

async fn make_default_sources_rs() -> anyhow::Result<()> {
    let mut file = BufWriter::new(File::create("includes/sources.array").await?);

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
    macro_rules! get_var {
        ($lit: literal) => {{
            println!("cargo::rerun-if-env-changed={}", $lit);
            env::var($lit).map_err(|e| io::Error::other(format!(concat!($lit, " {err}"), err = e)))
        }};
    }

    if get_var!("CARGO_CFG_TARGET_OS")? == "linux" {
        println!("cargo::rerun-if-changed=modules/linux-dispatcher");
        println!("cargo::rerun-if-changed=src/network_listener/linux/dispatcher");

        let target = get_var!("TARGET")?;
        let target = target.trim();

        let _ = tokio::fs::remove_dir_all(format!("./target/{target}/linux-dispatcher")).await;

        Command::new("cargo")
            .stdout(Stdio::from(io::stderr()))
            .stderr(Stdio::inherit())
            .args(["build", "--profile", "linux-dispatcher", "--target", target])
            .current_dir("./modules/linux-dispatcher")
            .status()
            .await?
            .success()
            .then_some(())
            .ok_or_else(|| io::Error::other("failed to run dispatcher build command"))?;

        let bin_path = {
            let path = format!("./target/{target}/linux-dispatcher/linux-dispatcher");

            Command::new("upx")
                .args(["--best", &*path])
                .stdout(Stdio::from(io::stderr()))
                .stderr(Stdio::inherit())
                .status()
                .await
                .map_err(|err| {
                    io::Error::other(format!(
                        "failed to run upx make sure upx is installed; {err}"
                    ))
                })?
                .success()
                .then_some(())
                .ok_or_else(|| {
                    io::Error::other("UPX failed to run successfully on linux-dispatcher")
                })?;

            tokio::fs::try_exists(&path)
                .await?
                .then_some(path)
                .ok_or_else(|| io::Error::other("unable to find dispatcher binary"))?
        };

        tokio::fs::copy(
            bin_path,
            PathBuf::from(get_var!("OUT_DIR")?).join("./dispatcher-bin"),
        )
        .await?;
    }

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tokio::fs::create_dir_all("./includes").await.unwrap();

    println!("cargo::rerun-if-changed=default");
    try_join!(
        make_default_sources_toml(),
        make_default_sources_rs(),
        generate_dispatcher()
    )
    .unwrap();
}
