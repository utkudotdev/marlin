// Copyright (C) 2024 Ethan Uppal.
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3 of the License only.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more
// details.
//
// You should have received a copy of the GNU General Public License along with
// this program.  If not, see <https://www.gnu.org/licenses/>.

use example_verilog_project::IncludeMain;
use marlin::{
    verilator::{VerilatorRuntime, VerilatorRuntimeOptions, verilator_version},
    verilog::prelude::*,
};
use snafu::Whatever;

#[test]
#[snafu::report]
fn basic_parameters() -> Result<(), Whatever> {
    let runtime = VerilatorRuntime::new2(
        "artifacts",
        &["src/include_for_width.sv"],
        &["src"],
        [],
        VerilatorRuntimeOptions::default()
            .allow_unsupported_verilator(Some(verilator_version!(5 020))),
    )?;

    let mut main = runtime.create_model_simple::<IncludeMain>()?;

    main.inp = 15;
    main.eval();
    assert_eq!(main.out, 15);

    Ok(())
}
