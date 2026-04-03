// Copyright (C) 2024 Ethan Uppal.
//
// This Source Code Form is subject to the terms of the Mozilla Public License,
// v. 2.0. If a copy of the MPL was not distributed with this file, You can
// obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::{env, fmt, process::Command};

use camino::Utf8PathBuf;
use marlin_verilator::{PortDeclaration, PortDirection};
use marlin_verilog_macro_builder::{MacroArgs, build_verilated_struct};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use serde::Deserialize;
use syn::{parse_macro_input, spanned::Spanned};

#[proc_macro_attribute]
pub fn verilog(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(args as MacroArgs);

    let module_params = args.module_params.unwrap_or_default();

    let json = match parse_verilog(
        &args.name,
        &args.source_path,
        &args.include_paths.unwrap_or_default(),
        &module_params,
    ) {
        Ok(json) => json,
        Err(error) => {
            return error.into();
        }
    };

    let ports = match extract_verilog_ports(&args.name, &json) {
        Ok(ports) => ports,
        Err(error) => {
            return error.into();
        }
    };

    build_verilated_struct(
        "verilog",
        &args.name,
        &args.source_path,
        ports,
        &module_params,
        item.into(),
    )
    .into()
}

// TODO: do we really need these to be syn objects
fn parse_verilog(
    top_name: &syn::LitStr,
    source_path: &syn::LitStr,
    include_directories: &Vec<syn::LitStr>,
    module_params: &HashMap<syn::Ident, i64>,
) -> Result<VerilatorJson, proc_macro2::TokenStream> {
    // TODO: we can probably do a lot of code sharing here? Should we cache these outputs? How bad is it to parse every time in a proc macro

    let resolved_source = resolve_input_path(source_path);
    let mut verilator_command = Command::new("verilator");
    verilator_command
        .arg(&resolved_source)
        .args(["--json-only", "--quiet"])
        .args(["--json-only-output", "/dev/stdout"])
        .args(["--json-only-meta-output", "/dev/null"])
        .args(["--top-module", top_name.value().as_str()]);

    for include_directory in include_directories {
        let resolved = resolve_input_path(include_directory);
        verilator_command.arg(format!("-I{resolved}"));
    }

    for (param, value) in module_params {
        verilator_command.arg(format!("-G{param}={value}"));
    }

    let verilator_output = verilator_command.output().map_err(|e| {
        syn::Error::new_spanned(
            top_name,
            format!("Verilator invocation failed! Error: {e}"),
        )
        .into_compile_error()
    })?;

    if !verilator_output.status.success() {
        return Err(syn::Error::new_spanned(top_name, format!(
            "Verilator invocation failed with nonzero exit code {}\n\n--- STDOUT ---\n{}\n\n--- STDERR ---\n{}",
            verilator_output.status,
            String::from_utf8(verilator_output.stdout).unwrap_or_default(),
            String::from_utf8(verilator_output.stderr).unwrap_or_default(),
        )).into_compile_error());
    }

    String::from_utf8(verilator_output.stdout)
        .map_err(|_| {
            syn::Error::new_spanned(
                top_name,
                "Failed to decode verilator output.",
            )
            .into_compile_error()
        })
        .and_then(|s| {
            serde_json::from_str(&s).map_err(|serde_err| {
                syn::Error::new_spanned(
                    top_name,
                    format!("Failed to parse verilator output: {serde_err}"),
                )
                .into_compile_error()
            })
        })
}

#[derive(Deserialize)]
struct VerilatorJson {
    modulesp: Vec<VerilatorJsonModule>,
    miscsp: Vec<VerilatorJsonInfo>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum VerilatorJsonModule {
    #[serde(rename = "MODULE")]
    Module {
        name: String,
        stmtsp: Vec<VerilatorJsonStatement>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum VerilatorJsonInfo {
    #[serde(rename = "TYPETABLE")]
    TypeTable { typesp: Vec<VerilatorJsonType> },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum VerilatorJsonStatement {
    #[serde(rename = "VAR")]
    Var(VerilatorJsonVar),
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum VerilatorJsonType {
    #[serde(rename = "BASICDTYPE")]
    BasicDType {
        addr: String,
        #[serde(default = "range_default")]
        range: String,
    },
    #[serde(other)]
    Unknown,
}

fn range_default() -> String {
    "1:0".to_owned()
}

#[derive(Deserialize)]
#[serde(tag = "varType")]
enum VerilatorJsonVar {
    #[serde(rename = "PORT")]
    Port {
        name: String,
        dtypep: String,
        direction: VerilatorJsonPortDir,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
enum VerilatorJsonPortDir {
    #[serde(rename = "OUTPUT")]
    Output,
    #[serde(rename = "INPUT")]
    Input,
    #[serde(rename = "INOUT")]
    Inout,
}

fn extract_verilog_ports<'json>(
    top_name: &syn::LitStr,
    json: &'json VerilatorJson,
) -> Result<Vec<PortDeclaration<'json>>, proc_macro2::TokenStream> {
    let ports = json
        .modulesp
        .iter()
        .filter_map(|m| match m {
            VerilatorJsonModule::Module { name, stmtsp }
                if *name == top_name.value() =>
            {
                Some(stmtsp)
            }
            _ => None,
        })
        .next()
        .ok_or(
            syn::Error::new_spanned(
                top_name,
                format!("No Verilog module named \"{}\"", top_name.value()),
            )
            .into_compile_error(),
        )?
        .iter()
        .filter_map(|s| match s {
            VerilatorJsonStatement::Var(VerilatorJsonVar::Port {
                name,
                dtypep,
                direction,
            }) => Some((name, dtypep, direction)),
            _ => None,
        });

    let type_map: HashMap<&str, &str> = json
        .miscsp
        .iter()
        .filter_map(|m| match m {
            VerilatorJsonInfo::TypeTable { typesp } => Some(typesp),
            _ => None,
        })
        .next()
        .expect("No type table found in Verilator output")
        .iter()
        .filter_map(|ty| match ty {
            VerilatorJsonType::BasicDType { addr, range } => {
                Some((addr.as_str(), range.as_str()))
            }
            _ => None,
        })
        .collect();

    let mut result = vec![];
    for (name, dtypep, direction) in ports {
        let range_str = type_map[dtypep.as_str()];

        let direction = match direction {
            VerilatorJsonPortDir::Input => PortDirection::Input,
            VerilatorJsonPortDir::Output => PortDirection::Output,
            VerilatorJsonPortDir::Inout => PortDirection::Inout,
        };

        let split: [&str; 2] = range_str
            .split(":")
            .collect::<Vec<&str>>()
            .try_into()
            .expect("Bad port width format");

        // TODO: improve error reporting here?
        let (msb, lsb) = split.map(&str::parse::<i64>).into();
        let msb_unwrap = msb.unwrap();
        let lsb_unwrap = lsb.unwrap();
        let width = (msb_unwrap - lsb_unwrap + 1).try_into().unwrap();

        result.push(PortDeclaration {
            name,
            lsb: lsb_unwrap.try_into().unwrap(),
            width,
            direction,
        })
    }

    Ok(result)
}

fn resolve_input_path(path: &syn::LitStr) -> Utf8PathBuf {
    let manifest_directory = Utf8PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("Please compile using `cargo` or set the `CARGO_MANIFEST_DIR` environment variable"));
    manifest_directory.join(path.value())
}

enum DPIPrimitiveType {
    Bool,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
}

impl fmt::Display for DPIPrimitiveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DPIPrimitiveType::Bool => "bool",
            DPIPrimitiveType::U8 => "u8",
            DPIPrimitiveType::U16 => "u16",
            DPIPrimitiveType::U32 => "u32",
            DPIPrimitiveType::U64 => "u64",
            DPIPrimitiveType::I8 => "i8",
            DPIPrimitiveType::I16 => "i16",
            DPIPrimitiveType::I32 => "i32",
            DPIPrimitiveType::I64 => "i64",
        }
        .fmt(f)
    }
}

impl DPIPrimitiveType {
    fn as_c(&self) -> &'static str {
        match self {
            DPIPrimitiveType::Bool => "svBit",
            DPIPrimitiveType::U8 => "uint8_t",
            DPIPrimitiveType::U16 => "uint16_t",
            DPIPrimitiveType::U32 => "uint32_t",
            DPIPrimitiveType::U64 => "uint64_t",
            DPIPrimitiveType::I8 => "int8_t",
            DPIPrimitiveType::I16 => "int16_t",
            DPIPrimitiveType::I32 => "int32_t",
            DPIPrimitiveType::I64 => "int64_t",
        }
    }
}

fn parse_dpi_primitive_type(
    ty: &syn::TypePath,
) -> Result<DPIPrimitiveType, syn::Error> {
    if let Some(qself) = &ty.qself {
        return Err(syn::Error::new_spanned(
            qself.lt_token,
            "Primitive integer type should not be qualified in DPI function",
        ));
    }

    match ty
        .path
        .require_ident()
        .or(Err(syn::Error::new_spanned(
            ty,
            "Primitive integer type should not have multiple path segments",
        )))?
        .to_string()
        .as_str()
    {
        "bool" => Ok(DPIPrimitiveType::Bool),
        "u8" => Ok(DPIPrimitiveType::U8),
        "u16" => Ok(DPIPrimitiveType::U16),
        "u32" => Ok(DPIPrimitiveType::U32),
        "u64" => Ok(DPIPrimitiveType::U64),
        "i8" => Ok(DPIPrimitiveType::I8),
        "i16" => Ok(DPIPrimitiveType::I16),
        "i32" => Ok(DPIPrimitiveType::I32),
        "i64" => Ok(DPIPrimitiveType::I64),
        _ => Err(syn::Error::new_spanned(
            ty,
            "Unknown primitive integer type",
        )),
    }
}

enum DPIType {
    Input(DPIPrimitiveType),
    /// Veriltor handles output and inout types the same
    Inout(DPIPrimitiveType),
}

fn parse_dpi_type(ty: &syn::Type) -> Result<DPIType, syn::Error> {
    match ty {
        syn::Type::Path(type_path) => {
            Ok(DPIType::Input(parse_dpi_primitive_type(type_path)?))
        }
        syn::Type::Reference(syn::TypeReference {
            and_token,
            lifetime,
            mutability,
            elem,
        }) => {
            if mutability.is_none() {
                return Err(syn::Error::new_spanned(
                    and_token,
                    "DPI output or inout type must be represented with a mutable reference",
                ));
            }
            if let Some(lifetime) = lifetime {
                return Err(syn::Error::new_spanned(
                    lifetime,
                    "DPI output or inout type cannot use lifetimes",
                ));
            }

            let syn::Type::Path(type_path) = elem.as_ref() else {
                return Err(syn::Error::new_spanned(
                    elem,
                    "DPI output or inout type must be a mutable reference to a primitive integer type",
                ));
            };
            Ok(DPIType::Inout(parse_dpi_primitive_type(type_path)?))
        }
        other => Err(syn::Error::new_spanned(
            other,
            "This type is not supported in DPI. Please use primitive integers or mutable references to them",
        )),
    }
}

/// Marlin allows you to import Rust functions into (System)Verilog over DPI.
/// The function must have "C" linkage and be imported into SystemVerilog with
/// "DPI-C" linkage.
///
/// For example:
/// ```ignore
/// // in Rust
/// #[verilog::dpi]
/// pub extern "C" fn three(out: &mut u32) {
///     *out = 3;
/// }
/// ```
/// ```systemverilog
/// // in SystemVerilog
/// import "DPI-C" function void three(output int out);
/// ```
///
/// The Rust function can only take in primitive integer types at or below
/// 64-bit width and booleans. The order and count of parameters must correspond
/// exactly with the SystemVerilog import declaration.
///
/// Any `input` parameter on the Verilog side should correspond to a plain
/// argument on the Rust side. Any `output` or `inout` parmaeter on the Verilog
/// side should corresponding to a mutable reference on the Rust side. Note that
/// the names of parmaeters on either side are irrelevant and need not
/// correspond.
///
/// Here are some examples:
///
/// | SystemVerilog parameter | Rust parameter |
/// | --- | --- |
/// | `output int foo` | `foo: &mut i32` |
/// | `input bit bar` | `bar: bool` |
///
/// ## More
///
/// Please reference the [Verilator docs on DPI](https://verilator.org/guide/latest/connecting.html#direct-programming-interface-dpi) for further information.
/// You can also see the [corresponding page in the Marlin handbook](https://www.ethanuppal.com/marlin/verilog/dpi.html).
#[proc_macro_attribute]
pub fn dpi(_args: TokenStream, item: TokenStream) -> TokenStream {
    let item_fn = parse_macro_input!(item as syn::ItemFn);

    if !matches!(item_fn.vis, syn::Visibility::Public(_)) {
        return syn::Error::new_spanned(
            item_fn.vis,
            "Marking the function `pub` is required to expose this Rust function to C",
        )
        .into_compile_error()
        .into();
    }

    let Some(abi) = &item_fn.sig.abi else {
        return syn::Error::new_spanned(
            item_fn,
            "`extern \"C\"` is required to expose this Rust function to C",
        )
        .into_compile_error()
        .into();
    };

    if !abi
        .name
        .as_ref()
        .map(|name| name.value().as_str() == "C")
        .unwrap_or(true)
    {
        return syn::Error::new_spanned(
            item_fn,
            "You must specify the C ABI for the `extern` marking",
        )
        .into_compile_error()
        .into();
    }

    if item_fn.sig.generics.lt_token.is_some() {
        return syn::Error::new_spanned(
            item_fn.sig.generics,
            "Generics are not supported for DPI functions",
        )
        .into_compile_error()
        .into();
    }

    if let Some(asyncness) = &item_fn.sig.asyncness {
        return syn::Error::new_spanned(
            asyncness,
            "DPI functions must be synchronous",
        )
        .into_compile_error()
        .into();
    }

    if let syn::ReturnType::Type(_, return_type) = &item_fn.sig.output {
        return syn::Error::new_spanned(
            return_type,
            "DPI functions cannot have a return value",
        )
        .into_compile_error()
        .into();
    }

    let ports =
        match item_fn
            .sig
            .inputs
            .iter()
            .try_fold(vec![], |mut ports, input| {
                let syn::FnArg::Typed(parameter) = input else {
                    return Err(syn::Error::new_spanned(
                        input,
                        "Invalid parameter on DPI function",
                    ));
                };

                let syn::Pat::Ident(name) = &*parameter.pat else {
                    return Err(syn::Error::new_spanned(
                        parameter,
                        "Function argument must be an identifier",
                    ));
                };

                let attrs = parameter.attrs.clone();
                ports.push((name, attrs, parse_dpi_type(&parameter.ty)?));
                Ok(ports)
            }) {
            Ok(ports) => ports,
            Err(error) => {
                return error.into_compile_error().into();
            }
        };

    let attributes = item_fn.attrs;
    let function_name = item_fn.sig.ident;
    let body = item_fn.block;

    let struct_name = format_ident!("__DPI_{}", function_name);

    let mut parameter_types = vec![];
    let mut parameters = vec![];

    for (name, attributes, dpi_type) in &ports {
        let parameter_type = match dpi_type {
            DPIType::Input(inner) => {
                let type_ident = format_ident!("{}", inner.to_string());
                quote! { #type_ident }
            }
            DPIType::Inout(inner) => {
                let type_ident = format_ident!("{}", inner.to_string());
                quote! { *mut #type_ident }
            }
        };
        parameter_types.push(parameter_type.clone());
        parameters.push(quote! {
            #(#attributes)* #name: #parameter_type
        });
    }

    let preamble =
        ports
            .iter()
            .filter_map(|(name, _, dpi_type)| match dpi_type {
                DPIType::Inout(_) => Some(quote! {
                    let #name = unsafe { &mut *#name };
                }),
                _ => None,
            });

    let function_name_literal = syn::LitStr::new(
        function_name.to_string().as_str(),
        function_name.span(),
    );

    let c_signature = ports
        .iter()
        .map(|(name, _, dpi_type)| {
            let c_type = match dpi_type {
                DPIType::Input(inner) => inner.as_c().to_string(),
                DPIType::Inout(inner) => format!("{}*", inner.as_c()),
            };
            let name_literal =
                syn::LitStr::new(name.ident.to_string().as_str(), name.span());
            let type_literal = syn::LitStr::new(&c_type, name.span());
            quote! {
                (#name_literal, #type_literal)
            }
        })
        .collect::<Vec<_>>();

    quote! {
        #[allow(non_camel_case_types)]
        struct #struct_name;

        impl #struct_name {
            #(#attributes)*
            pub extern "C" fn call(#(#parameters),*) {
                #(#preamble)*
                #body
            }
        }

        impl verilog::__reexports::verilator::dpi::DpiFunction for #struct_name {
            fn name(&self) -> &'static str {
                #function_name_literal
            }

            fn signature(&self) -> &'static [(&'static str, &'static str)] {
                &[#(#c_signature),*]
            }

            fn pointer(&self) -> *const std::ffi::c_void {
                #struct_name::call as extern "C" fn(#(#parameter_types),*) as *const std::ffi::c_void
            }
        }

        #[allow(non_upper_case_globals)]
        pub static #function_name: &'static dyn verilog::__reexports::verilator::dpi::DpiFunction = &#struct_name;
    }
    .into()
}
