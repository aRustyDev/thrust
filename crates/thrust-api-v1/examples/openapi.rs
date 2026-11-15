//! Prints the OpenAPI document of `/api/v1` on stdout (issue #58): the operations that are described so far, written from the types
//! the routes answer with. `scripts/check-openapi.sh` runs this, compares the result with the committed copy, and diffs it against
//! the pinned Plane schema with `oasdiff`.
//!
//! `cargo run -q -p thrust-api-v1 --features openapi --example openapi`

use std::process::ExitCode;

fn main() -> ExitCode {
    match thrust_api_v1::openapi::openapi().to_pretty_json() {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("the document does not serialize: {e}");
            ExitCode::FAILURE
        }
    }
}
