// Copyright (C) 2024 Ethan Uppal.
//
// This Source Code Form is subject to the terms of the Mozilla Public License,
// v. 2.0. If a copy of the MPL was not distributed with this file, You can
// obtain one at https://mozilla.org/MPL/2.0/.

use core::panic;
use std::collections::HashMap;

use sv_parser::{self as sv};

pub fn evaluate_numeric_constant_expression(
    ast: &sv::SyntaxTree,
    expression: &sv::ConstantExpression,
    paramater_map: &HashMap<&str, i64>,
) -> i64 {
    match expression {
        sv::ConstantExpression::ConstantPrimary(constant_primary) => {
            match &**constant_primary {
                sv::ConstantPrimary::PrimaryLiteral(primary_literal) => {
                    match &**primary_literal {
                        sv::PrimaryLiteral::Number(number) => match &**number {
                            sv::Number::IntegralNumber(integral_number) => {
                                match &**integral_number {
                                    sv::IntegralNumber::DecimalNumber(
                                        decimal_number,
                                    ) => match &**decimal_number {
                                        sv::DecimalNumber::UnsignedNumber(
                                            unsigned_number,
                                        ) => ast
                                            .get_str_trim(
                                                &unsigned_number.nodes.0,
                                            )
                                            .unwrap()
                                            .parse()
                                            .unwrap(),
                                        sv::DecimalNumber::BaseUnsigned(
                                            _decimal_number_base_unsigned,
                                        ) => todo!(),
                                        sv::DecimalNumber::BaseXNumber(
                                            _decimal_number_base_xnumber,
                                        ) => todo!(),
                                        sv::DecimalNumber::BaseZNumber(
                                            _decimal_number_base_znumber,
                                        ) => todo!(),
                                    },
                                    sv::IntegralNumber::OctalNumber(
                                        _octal_number,
                                    ) => todo!(),
                                    sv::IntegralNumber::BinaryNumber(
                                        _binary_number,
                                    ) => todo!(),
                                    sv::IntegralNumber::HexNumber(
                                        _hex_number,
                                    ) => todo!(),
                                }
                            }
                            sv::Number::RealNumber(_real_number) => {
                                panic!("Real number")
                            }
                        },
                        _ => todo!("Other constant primary literals"),
                    }
                }
                sv::ConstantPrimary::PsParameter(_) => panic!("Hi"),
                sv::ConstantPrimary::Specparam(_) => panic!("Hi2"),
                sv::ConstantPrimary::ConstantFunctionCall(
                    constant_function_call,
                ) => evaluate_constant_function_call(
                    ast,
                    &constant_function_call,
                    paramater_map,
                ),
                v => {
                    todo!("Other types of constant primary expression")
                }
            }
        }
        sv::ConstantExpression::Unary(_constant_expression_unary) => {
            todo!("Constant unary expressions")
        }
        sv::ConstantExpression::Binary(constant_expression_binary) => {
            let (left, op, attrs, right) = &constant_expression_binary.nodes;
            match attrs[..] {
                [] => {
                    let left_value = evaluate_numeric_constant_expression(
                        ast,
                        left,
                        paramater_map,
                    );

                    let right_value = evaluate_numeric_constant_expression(
                        ast,
                        right,
                        paramater_map,
                    );

                    let op_str = ast.get_str_trim(&op.nodes.0).unwrap();

                    match op_str {
                        "-" => left_value - right_value,
                        _ => todo!("Binary operators other than subtract"),
                    }
                }
                _ => todo!("Constant binary expression attributes"),
            }
        }
        sv::ConstantExpression::Ternary(_constant_expression_ternary) => {
            todo!("Constant ternary expressions")
        }
        sv::ConstantExpression::Inside(_constant_expression_inside) => {
            todo!("Constant inside expressions")
        }
    }
}

// Due to ambiguity in the BNF sv-parser does not handle correctly,
// constants are parsed as constant function calls, even though they are not.
pub fn evaluate_constant_function_call(
    ast: &sv::SyntaxTree,
    constant_function_call: &sv::ConstantFunctionCall,
    paramater_map: &HashMap<&str, i64>,
) -> i64 {
    match &constant_function_call.nodes {
        (function_subroutine_call,) => match &function_subroutine_call.nodes {
            (sv::SubroutineCall::TfCall(tf_call),) => {
                let (ps_ident, attrs, args) = &tf_call.nodes;

                if attrs.len() != 0 {
                    panic!("Function call attributes are not supported.")
                }

                if let Some(_) = args {
                    panic!(
                        "Constant function calls with actual arguments are not supported. \
                        Only module parameter usage, which happens to be parsed as a \
                        constant function call, is allowed."
                    );
                }

                match ps_ident {
                    sv::PsOrHierarchicalTfIdentifier::PackageScope(
                        package_scope,
                    ) => match &package_scope.nodes {
                        (None, tf_ident) => {
                            let name = ast.get_str_trim(tf_ident).unwrap();
                            paramater_map[name]
                        }
                        _ => panic!(
                            "Specified package scope is not supported. Only top-level module \
                            parameters may be used in port declarations."
                        ),
                    },
                    _ => panic!(
                        "Hierarchical identifiers are not supported. Only top-level module \
                        parameters may be used in port declarations."
                    ),
                }
            }
            _ => todo!(),
        },
    }
}
