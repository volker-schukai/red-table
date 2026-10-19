use std::{
    io::{self, IsTerminal},
    process::ExitCode,
};

fn main() -> ExitCode {
    match red_table::run_from(std::env::args_os().skip(1)) {
        Ok(outcome) => ExitCode::from(outcome.exit_code()),
        Err(error) => {
            let stderr = io::stderr();
            let color = red_table::diagnostics::use_color(
                stderr.is_terminal(),
                std::env::var_os("TERM").as_deref(),
                std::env::var_os("NO_COLOR").as_deref(),
            );
            let _ = red_table::diagnostics::write_error(&mut stderr.lock(), error.as_ref(), color);
            ExitCode::FAILURE
        }
    }
}
