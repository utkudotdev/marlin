// Copyright (C) 2024 Ethan Uppal.
//
// This Source Code Form is subject to the terms of the Mozilla Public License,
// v. 2.0. If a copy of the MPL was not distributed with this file, You can
// obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use marlin_verilator::{
    PortDeclaration, PortDirection,
    compute_wdata_word_count_from_width_not_msb,
    ffi_names::{
        TRACE_CLOSE_AND_DELETE, TRACE_DUMP, TRACE_FLUSH, TRACE_OPEN_NEXT,
    },
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub struct MacroArgs {
    pub source_path: syn::LitStr,
    pub name: syn::LitStr,

    // TODO: should probably parse as smth else
    pub include_paths: Option<Vec<syn::LitStr>>,

    pub module_params: Option<HashMap<syn::Ident, i64>>,

    /// Deprecated; does nothing.
    pub clock_port: Option<syn::LitStr>,
    /// Deprecated; does nothing.
    pub reset_port: Option<syn::LitStr>,
}

impl syn::parse::Parse for MacroArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        syn::custom_keyword!(src);
        syn::custom_keyword!(name);
        syn::custom_keyword!(includes);

        // TODO: instead of doing this, should we find a way to generate a generic struct?
        syn::custom_keyword!(params);

        syn::custom_keyword!(clock);
        syn::custom_keyword!(reset);
        input.parse::<src>()?;
        input.parse::<syn::Token![=]>()?;
        let source_path = input.parse::<syn::LitStr>()?;

        input.parse::<syn::Token![,]>()?;

        input.parse::<name>()?;
        input.parse::<syn::Token![=]>()?;
        let name = input.parse::<syn::LitStr>()?;

        let mut include_paths = None;
        let mut clock_port = None;
        let mut reset_port = None;
        let mut module_params = None;

        while input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;

            let lookahead = input.lookahead1();
            if lookahead.peek(includes) {
                input.parse::<includes>()?;
                input.parse::<syn::Token![=]>()?;

                let content;
                let _ = syn::bracketed!(content in input);

                let paths = syn::punctuated::Punctuated::<
                    syn::LitStr,
                    syn::Token![,],
                >::parse_terminated(&content)?
                .into_iter()
                .collect();

                include_paths = Some(paths);
            } else if lookahead.peek(clock) {
                input.parse::<clock>()?;
                input.parse::<syn::Token![=]>()?;
                clock_port = Some(input.parse::<syn::LitStr>()?);
            } else if lookahead.peek(reset) {
                input.parse::<reset>()?;
                input.parse::<syn::Token![=]>()?;
                reset_port = Some(input.parse::<syn::LitStr>()?);
            } else if lookahead.peek(params) {
                input.parse::<params>()?;
                input.parse::<syn::Token![=]>()?;

                let content;
                let _ = syn::braced!(content in input);

                let map = content
                    .parse_terminated(parse_key_value, syn::Token![,])?
                    .into_iter()
                    .collect();
                module_params = Some(map);
            } else {
                return Err(lookahead.error());
            }
        }

        Ok(Self {
            source_path,
            name,
            include_paths,
            module_params,
            clock_port,
            reset_port,
        })
    }
}

fn parse_key_value(
    input: syn::parse::ParseStream,
) -> syn::Result<(syn::Ident, i64)> {
    let lookahead = input.lookahead1();

    let key = input.parse()?;

    input.parse::<syn::Token![:]>()?;

    let value = if lookahead.peek(syn::Token![-]) {
        let _ = input.parse::<syn::Token![-]>()?;
        -input.parse::<syn::LitInt>()?.base10_parse::<i64>()?
    } else {
        input.parse::<syn::LitInt>()?.base10_parse::<i64>()?
    };

    Ok((key, value))
}

pub fn build_verilated_struct(
    macro_name: &str,
    top_name: &syn::LitStr,
    source_path: &syn::LitStr,
    verilog_ports: Vec<PortDeclaration>,
    verilog_params: &HashMap<syn::Ident, i64>,
    item: TokenStream,
) -> TokenStream {
    let crate_name = format_ident!("{}", macro_name);
    let item = match syn::parse::<syn::ItemStruct>(item.into()) {
        Ok(item) => item,
        Err(error) => {
            return error.into_compile_error();
        }
    };

    let mut struct_members = vec![];

    let mut preeval_impl = vec![];
    let mut posteval_impl = vec![];

    let mut verilated_model_ports_impl = vec![];
    let mut verilated_model_init_impl = vec![];
    let mut verilated_model_init_self = vec![];

    let mut dynamic_read_arms = vec![];
    let mut dynamic_pin_arms = vec![];

    verilated_model_init_impl.push(quote! {
        let new_model: extern "C" fn() -> *mut std::ffi::c_void =
            *unsafe { library.get(concat!("ffi_new_V", #top_name).as_bytes()) }
                .expect("failed to get symbol");
        let model = (new_model)();

        let eval_model: extern "C" fn(*mut std::ffi::c_void) =
            *unsafe { library.get(concat!("ffi_V", #top_name, "_eval").as_bytes()) }
                .expect("failed to get symbol");
    });
    verilated_model_init_self.push(quote! {
        eval_model,
        _internal_model: model,
        _marker: std::marker::PhantomData
    });

    for port in verilog_ports {
        let PortDeclaration {
            name: port_name,
            direction: port_direction,
            lsb: port_lsb,
            width: port_width,
        } = port;
        let port_msb = port_lsb + port_width - 1;

        if port_name.chars().any(|c| c == '\\' || c == ' ') {
            return syn::Error::new_spanned(
                top_name,
                "Escaped module names are not supported",
            )
            .into_compile_error();
        }

        let verilator_interface_port_type_name = if port_width <= 8 {
            quote! { CData }
        } else if port_width <= 16 {
            quote! { SData }
        } else if port_width <= 32 {
            quote! { IData }
        } else if port_width <= 64 {
            quote! { QData }
        } else {
            match port_direction {
                PortDirection::Input => {
                    quote! { WDataInP }
                }
                PortDirection::Output => {
                    quote! { WDataOutP }
                }
                PortDirection::Inout => {
                    todo!("Inout wide ports are not currently supported")
                }
            }
        };
        let verilator_interface_port_type = quote! {
            #crate_name::__reexports::verilator::types::#verilator_interface_port_type_name
        };

        let port_type_with_generics = if port_width <= 64 {
            verilator_interface_port_type.clone()
        } else {
            let length =
                compute_wdata_word_count_from_width_not_msb(port_width);
            match port_direction {
                PortDirection::Input => {
                    quote! { #crate_name::__reexports::verilator::WideIn<#length> }
                }
                PortDirection::Output => {
                    quote! { #crate_name::__reexports::verilator::WideOut<#length> }
                }

                PortDirection::Inout => {
                    todo!("Inout wide ports are not currently supported")
                }
            }
        };
        let port_type_without_generics = if port_width <= 64 {
            verilator_interface_port_type.clone()
        } else {
            match port_direction {
                PortDirection::Input => {
                    quote! { #crate_name::__reexports::verilator::WideIn }
                }
                PortDirection::Output => {
                    quote! { #crate_name::__reexports::verilator::WideOut }
                }

                PortDirection::Inout => {
                    todo!("Inout wide ports are not currently supported")
                }
            }
        };

        let port_name_ident = format_ident!("{}", port_name);
        let port_documentation = syn::LitStr::new(
            &format!(
                "Corresponds to Verilog `{} {}[{}:{}]`.",
                port_direction, port_name, port_msb, port_lsb
            ),
            top_name.span(),
        );
        struct_members.push(quote! {
            #[doc = #port_documentation]
            pub #port_name_ident: #port_type_with_generics
        });
        if port_width <= 64 {
            verilated_model_init_self.push(quote! {
                #port_name_ident: 0 as _
            });
        } else {
            verilated_model_init_self.push(quote! {
                #port_name_ident: std::default::Default::default()
            });
        }

        let port_name_literal = syn::LitStr::new(port_name, top_name.span());

        match port_direction {
            PortDirection::Input => {
                let setter = format_ident!("pin_{}", port_name);
                struct_members.push(quote! {
                    #[doc(hidden)]
                    #setter: extern "C" fn(*mut std::ffi::c_void, #verilator_interface_port_type)
                });
                if port_width <= 64 {
                    preeval_impl.push(quote! {
                        (self.#setter)(self._internal_model, self.#port_name_ident);
                    });
                } else {
                    preeval_impl.push(quote! {
                        (self.#setter)(self._internal_model, self.#port_name_ident.as_ptr());
                    });
                }

                verilated_model_init_impl.push(quote! {
                    let #setter: extern "C" fn(*mut std::ffi::c_void, #verilator_interface_port_type) =
                        *unsafe { library.get(concat!("ffi_V", #top_name, "_pin_", #port_name).as_bytes()) }
                            .expect("failed to get symbol");
                });
                verilated_model_init_self.push(quote! { #setter });

                if port_width <= 64 {
                    dynamic_pin_arms.push(quote! {
                        #port_name_literal => {
                            if let #crate_name::__reexports::verilator::dynamic::VerilatorValue::#verilator_interface_port_type_name(inner) = value {
                                self.#port_name_ident = inner;
                            } else {
                                return Err(
                                    #crate_name::__reexports::verilator::dynamic::DynamicVerilatedModelError::InvalidPortWidth {
                                        top_module: Self::name().to_string(),
                                        port,
                                        width: #port_width as _,
                                        attempted_lower: 0,
                                        attempted_higher: value.width()
                                    },
                                );
                            }
                        }
                    });
                } else {
                    dynamic_pin_arms.push(quote! {
                        #port_name_literal => {
                            if let #crate_name::__reexports::verilator::dynamic::VerilatorValue::#verilator_interface_port_type_name(inner) = value {
                                let array = inner.try_into().map_err(|_| {
                                    #crate_name::__reexports::verilator::dynamic::DynamicVerilatedModelError::InvalidPortWidth {
                                        top_module: Self::name().to_string(),
                                        port,
                                        width: #port_width as _,
                                        attempted_lower: 0,
                                        attempted_higher: value.width()
                                    }
                                })?;
                                self.#port_name_ident = #port_type_without_generics::new(array);
                            } else {
                                return Err(
                                    #crate_name::__reexports::verilator::dynamic::DynamicVerilatedModelError::InvalidPortWidth {
                                        top_module: Self::name().to_string(),
                                        port,
                                        width: #port_width as _,
                                        attempted_lower: 0,
                                        attempted_higher: value.width()
                                    },
                                );
                            }
                        }
                    });
                }
            }
            PortDirection::Output => {
                let getter = format_ident!("read_{}", port_name);
                struct_members.push(quote! {
                    #[doc(hidden)]
                    #getter: extern "C" fn(*mut std::ffi::c_void) -> #verilator_interface_port_type
                });
                if port_width <= 64 {
                    posteval_impl.push(quote! {
                        self.#port_name_ident = (self.#getter)(self._internal_model);
                    });
                } else {
                    posteval_impl.push(quote! {
                        self.#port_name_ident = #port_type_without_generics::from_ptr((self.#getter)(self._internal_model));
                    });
                }

                verilated_model_init_impl.push(quote! {
                    let #getter: extern "C" fn(*mut std::ffi::c_void) -> #verilator_interface_port_type =
                        *unsafe { library.get(concat!("ffi_V", #top_name, "_read_", #port_name).as_bytes()) }
                            .expect("failed to get symbol");
                });
                verilated_model_init_self.push(quote! { #getter });

                dynamic_read_arms.push(quote! {
                    #port_name_literal => Ok(self.#port_name_ident.clone().into())
                });
            }
            _ => todo!("Unhandled port direction"),
        }

        let verilated_model_port_direction = match port_direction {
            PortDirection::Input => {
                quote! { #crate_name::__reexports::verilator::PortDirection::Input }
            }
            PortDirection::Output => {
                quote! { #crate_name::__reexports::verilator::PortDirection::Output }
            }
            _ => todo!("Other port directions"),
        };

        verilated_model_ports_impl.push(quote! {
            #crate_name::__reexports::verilator::PortDeclaration {
                name: #port_name,
                direction: #verilated_model_port_direction,
                lsb: #port_lsb,
                width: #port_width
            }
        });
    }

    let mut verilated_model_params_impl = vec![];
    for (ident, value) in verilog_params {
        let as_string = ident.to_string();
        verilated_model_params_impl.push(quote! { (#as_string, #value) });
    }

    struct_members.push(quote! {
        #[doc(hidden)]
        eval_model: extern "C" fn(*mut std::ffi::c_void)
    });

    let struct_name = item.ident;
    let vis = item.vis;
    let port_count = verilated_model_ports_impl.len();
    let param_count = verilated_model_params_impl.len();

    quote! {
        #vis struct #struct_name<'ctx> {
            #[doc(hidden)]
            _internal_trace_api: Option<#crate_name::__reexports::verilator::tracing::__private::TraceApi>,
            #[doc(hidden)]
            _internal_opened_trace: bool,
            #(#struct_members),*,
            #[doc = "# Safety\nThe Rust binding to the model will not outlive the runtime this model was created from (with lifetime `'ctx`) and is dropped when the runtime is."]
            #[doc(hidden)]
            _internal_model: *mut std::ffi::c_void,
            #[doc(hidden)]
            _marker: std::marker::PhantomData<&'ctx ()>,
            #[doc(hidden)]
            _unsend_unsync: std::marker::PhantomData<(std::cell::Cell<()>, std::sync::MutexGuard<'static, ()>)>
        }

        impl<'ctx> #crate_name::__reexports::verilator::tracing::OpenTrace<'ctx> for #struct_name<'ctx> {
            fn open_trace(
                &mut self,
                path: impl std::convert::AsRef<std::path::Path>,
            ) -> #crate_name::__reexports::verilator::tracing::Trace<'ctx> {
                let path = path.as_ref();
                if let Some(trace_api) = &self._internal_trace_api {
                    if self._internal_opened_trace {
                        panic!("Verilator does not support opening multiple traces (see issue #5813). You can instead split the already-opened trace if you are using a VCD.");
                    }
                    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).expect("Failed to convert provided VCD path to C string");
                    let trace_ptr = (trace_api.open_trace)(self._internal_model, c_path.as_ptr());
                    self._internal_opened_trace = true;
                    #crate_name::__reexports::verilator::tracing::__private::new_trace(
                        trace_ptr,
                        trace_api.dump,
                        trace_api.open_next,
                        trace_api.flush,
                        trace_api.close_and_delete
                    )
                } else {
                    #crate_name::__reexports::verilator::tracing::__private::new_trace_useless()
                }
            }
        }

        impl<'ctx> #crate_name::__reexports::verilator::AsVerilatedModel<'ctx> for #struct_name<'ctx> {
            fn name() -> &'static str {
                #top_name
            }

            fn source_path() -> &'static str {
                #source_path
            }

            fn ports() -> &'static [#crate_name::__reexports::verilator::PortDeclaration<'static>] {
                static PORTS: [#crate_name::__reexports::verilator::PortDeclaration<'static>; #port_count] = [#(#verilated_model_ports_impl),*];
                &PORTS
            }

            fn parameters() -> &'static [(&'static str, i64)] {
                static PARAMS: [(&'static str, i64); #param_count] = [#(#verilated_model_params_impl),*];
                &PARAMS
            }

            fn init_from(library: &'ctx #crate_name::__reexports::libloading::Library, tracing_enabled: bool) -> Self {
                #(#verilated_model_init_impl)*

                let trace_api =
                    if tracing_enabled {
                        use #crate_name::__reexports::verilator::tracing::__private::TraceApi;

                        let open_trace: extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char) -> *mut std::ffi::c_void =
                            *unsafe { library.get(concat!("ffi_V", #top_name, "_open_trace").as_bytes()).expect("failed to get open_trace symbol") };
                        let dump: extern "C" fn(*mut std::ffi::c_void, u64) =
                            *unsafe { library.get(#TRACE_DUMP.as_bytes()).expect("failed to get dump symbol") };
                        let open_next: extern "C" fn(*mut std::ffi::c_void, bool) =
                            *unsafe { library.get(#TRACE_OPEN_NEXT.as_bytes()).expect("failed to get open_next symbol") };
                        let flush: extern "C" fn(*mut std::ffi::c_void) =
                            *unsafe { library.get(#TRACE_FLUSH.as_bytes()).expect("failed to get flush symbol") };
                        let close_and_delete: extern "C" fn(*mut std::ffi::c_void) =
                            *unsafe { library.get(#TRACE_CLOSE_AND_DELETE.as_bytes()).expect("failed to get close_and_delete symbol") };
                        Some(TraceApi { open_trace, dump, open_next, flush, close_and_delete })
                    } else {
                        None
                    };

                Self {
                    _internal_trace_api: trace_api,
                    _internal_opened_trace: false,
                    #(#verilated_model_init_self),*,
                    _unsend_unsync: std::marker::PhantomData
                }
            }

            unsafe fn model(&self) -> *mut std::ffi::c_void {
                self._internal_model
            }
        }

        impl<'ctx> #crate_name::__reexports::verilator::AsDynamicVerilatedModel<'ctx> for #struct_name<'ctx> {
            fn eval(&mut self) {
                #(#preeval_impl)*
                (self.eval_model)(self._internal_model);
                #(#posteval_impl)*
            }

            fn read(
                &self,
                port: impl Into<String>,
            ) -> Result<#crate_name::__reexports::verilator::dynamic::VerilatorValue, #crate_name::__reexports::verilator::dynamic::DynamicVerilatedModelError> {
                use #crate_name::__reexports::verilator::AsVerilatedModel;

                let port = port.into();

                match port.as_str() {
                    #(#dynamic_read_arms,)*
                    _ => Err(#crate_name::__reexports::verilator::dynamic::DynamicVerilatedModelError::NoSuchPort {
                        top_module: Self::name().to_string(),
                        port,
                        source: None,
                    })
                }
            }

            fn pin(
                &mut self,
                port: impl Into<String>,
                value: impl Into<#crate_name::__reexports::verilator::dynamic::VerilatorValue<'ctx>>,
            ) -> Result<(), #crate_name::__reexports::verilator::dynamic::DynamicVerilatedModelError> {
                use #crate_name::__reexports::verilator::AsVerilatedModel;

                let port = port.into();
                let value = value.into();

                match port.as_str() {
                    #(#dynamic_pin_arms,)*
                    _ => {
                        return Err(#crate_name::__reexports::verilator::dynamic::DynamicVerilatedModelError::NoSuchPort {
                            top_module: Self::name().to_string(),
                            port,
                            source: None,
                        });
                    }
                }

                #[allow(unreachable_code)]
                Ok(())
            }
        }
    }
}
