use criterion::{Criterion, criterion_group, criterion_main};
use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_sandbox::{
    BubblewrapProvider, CommandSpec, LocalSandboxProvider, SandboxContext, SandboxPolicy,
    SandboxProvider,
};
use std::path::PathBuf;
use tokio::runtime::Runtime;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn command_spec() -> CommandSpec {
    let mut spec = CommandSpec::new("/usr/bin/printf");
    spec.args.push("ok".to_string());
    spec
}

fn sandbox_context() -> SandboxContext {
    SandboxContext {
        workspace_root: workspace_root(),
        mode: SandboxMode::WorkspaceWrite,
        policy: SandboxPolicy::default(),
    }
}

fn bench_local_run_command(criterion: &mut Criterion) {
    let ctx = sandbox_context();
    let spec = command_spec();
    let runtime = Runtime::new().expect("tokio runtime");
    let provider = LocalSandboxProvider::new();
    let handle = runtime
        .block_on(provider.prepare(&ctx))
        .expect("local sandbox prepare");

    criterion.bench_function("sandbox_local_run_command", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(provider.run_command(&handle, spec.clone()))
                .expect("local run_command");
        });
    });

    runtime.block_on(provider.shutdown(handle));
}

fn bench_bubblewrap_run_command(criterion: &mut Criterion) {
    let provider = match BubblewrapProvider::new() {
        Ok(provider) => provider,
        Err(error) => {
            eprintln!("Skipping bubblewrap benchmark: {error}");
            return;
        }
    };

    let ctx = sandbox_context();
    let spec = command_spec();
    let runtime = Runtime::new().expect("tokio runtime");
    let handle = runtime
        .block_on(provider.prepare(&ctx))
        .expect("bubblewrap sandbox prepare");

    criterion.bench_function("sandbox_bubblewrap_run_command", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(provider.run_command(&handle, spec.clone()))
                .expect("bubblewrap run_command");
        });
    });

    runtime.block_on(provider.shutdown(handle));
}

criterion_group!(
    sandbox_benches,
    bench_local_run_command,
    bench_bubblewrap_run_command
);
criterion_main!(sandbox_benches);
