use oxc_allocator::CloneIn;
use oxc_ast::ast::*;
use oxc_ast::NONE;
use oxc_span::{GetSpan, SPAN};
use oxc_traverse::TraverseCtx;

use crate::component::{Import, ImportId, QWIK_CORE_SOURCE};
use crate::transform::generator::TransformGenerator;

use super::move_expression;

pub fn exit_jsx_child<'a>(
    gen: &mut TransformGenerator<'a>,
    node: &mut JSXChild<'a>,
    ctx: &mut TraverseCtx<'a, ()>,
) {
    if !gen.options.transpile_jsx {
        return;
    }
    gen.debug("EXIT: JSX child", ctx);

    let prop_wrap_key: Option<String> = if let JSXChild::ExpressionContainer(container) = node {
        if let Some(expr) = container.expression.as_expression() {
            gen.should_wrap_prop(expr).map(|(_, key)| key)
        } else {
            None
        }
    } else {
        None
    };

    let needs_signal_wrap: bool = if prop_wrap_key.is_none() {
        if let JSXChild::ExpressionContainer(container) = node {
            if let Some(expr) = container.expression.as_expression() {
                gen.should_wrap_signal_value(expr)
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if let Some(jsx) = gen.jsx_stack.last_mut() {
        let maybe_child = match node {
            JSXChild::Text(b) => {
                let text: &'a str = gen.builder.allocator.alloc_str(b.value.trim());
                if text.is_empty() {
                    None
                } else {
                    Some(
                        gen.builder
                            .expression_string_literal((*b).span, text, Some(text.into()))
                            .into(),
                    )
                }
            }
            JSXChild::Element(_) => Some(gen.replace_expr.take().unwrap().into()),
            JSXChild::Fragment(_) => Some(gen.replace_expr.take().unwrap().into()),
            JSXChild::ExpressionContainer(b) => {
                if !(*b).expression.is_expression() {
                    None
                } else {
                    jsx.static_subtree = false;
                    let expr = (*b).expression.to_expression_mut();
                    let span = expr.span();

                if let Some(prop_key) = &prop_wrap_key {
                    gen.needs_wrap_prop_import = true;
                    let prop_key_str: &'a str = ctx.ast.allocator.alloc_str(prop_key);
                    Some(
                        ctx.ast
                            .expression_call(
                                span,
                                ctx.ast.expression_identifier(SPAN, "_wrapProp"),
                                NONE,
                                ctx.ast.vec_from_array([
                                    Argument::from(ctx.ast.expression_identifier(SPAN, "_rawProps")),
                                    Argument::from(ctx.ast.expression_string_literal(
                                        SPAN,
                                        prop_key_str,
                                        None,
                                    )),
                                ]),
                                false,
                            )
                            .into(),
                    )
                } else if crate::inlined_fn::is_compound_signal_expression(expr) {
                    // Compound signal expression like (a || b).value - use _fnSignal with hoisted function
                    // Use convert_compound_signal_fn to handle the AST transformation properly
                    let captures = crate::inlined_fn::extract_compound_signal_captures(expr);

                    if !captures.is_empty() {
                        // Generate hoisted function: (p0, p1) => (p0 || p1).value
                        let counter = gen.hoisted_fn_counter;
                        let hf_name = format!("_hf{}", counter);
                        let hf_str_name = format!("{}_str", &hf_name);
                        gen.hoisted_fn_counter += 1;

                        // Use AST transformation to properly replace identifiers
                        let (hoisted_fn_code, hoisted_fn_str) = crate::inlined_fn::convert_compound_signal_fn(
                            expr,
                            &captures,
                            ctx.ast.allocator,
                        );

                        // Store in component_hoisted_fns for emission in entry point file
                        if let Some(hf_stack) = gen.component_hoisted_fns.last_mut() {
                            hf_stack.push((hf_name.clone(), hoisted_fn_code, hoisted_fn_str));
                        }

                        gen.needs_fn_signal_import = true;

                        if let Some(import_set) = gen.import_stack.last_mut() {
                            import_set.insert(Import::new(
                                vec![ImportId::Named("_fnSignal".into())],
                                QWIK_CORE_SOURCE,
                            ));
                        }

                        // Create captures array with original variable references
                        let captures_array = ctx.ast.expression_array(
                            SPAN,
                            ctx.ast.vec_from_iter(captures.iter().map(|name| {
                                ArrayExpressionElement::from(ctx.ast.expression_identifier(SPAN, ctx.ast.atom(name)))
                            })),
                        );

                        // Create _fnSignal call: _fnSignal(_hf0, [count, count2], _hf0_str)
                        Some(
                            ctx.ast.expression_call(
                                span,
                                ctx.ast.expression_identifier(SPAN, "_fnSignal"),
                                NONE,
                                ctx.ast.vec_from_array([
                                    Argument::from(ctx.ast.expression_identifier(SPAN, ctx.ast.atom(&hf_name))),
                                    Argument::from(captures_array),
                                    Argument::from(ctx.ast.expression_identifier(SPAN, ctx.ast.atom(&hf_str_name))),
                                ]),
                                false,
                            )
                            .into(),
                        )
                    } else {
                        Some(move_expression(&gen.builder, expr).into())
                    }
                } else if needs_signal_wrap {
                    gen.needs_wrap_prop_import = true;
                    if let Expression::StaticMemberExpression(static_member) = expr {
                        let signal_expr = static_member.object.clone_in(ctx.ast.allocator);
                        Some(
                            ctx.ast
                                .expression_call(
                                    span,
                                    ctx.ast.expression_identifier(SPAN, "_wrapProp"),
                                    NONE,
                                    ctx.ast.vec1(Argument::from(signal_expr)),
                                    false,
                                )
                                .into(),
                        )
                    } else {
                        Some(move_expression(&gen.builder, expr).into())
                    }
                } else if gen.loop_depth > 0 {
                    let iteration_vars = gen.iteration_var_stack.last().cloned().unwrap_or_default();

                    // First, check for simple member access like `btn.name` where btn is an iteration var.
                    // qwik-core uses _wrapProp(btn, "name") for these patterns instead of _fnSignal.
                    if let Some((obj_name, prop_name)) = crate::inlined_fn::is_simple_member_access(expr, &iteration_vars) {
                        gen.needs_wrap_prop_import = true;
                        // Also add to import_stack for segment file imports
                        if let Some(import_set) = gen.import_stack.last_mut() {
                            import_set.insert(Import::new(
                                vec![ImportId::Named("_wrapProp".into())],
                                QWIK_CORE_SOURCE,
                            ));
                        }
                        let prop_name_str: &'a str = ctx.ast.allocator.alloc_str(&prop_name);
                        Some(
                            ctx.ast
                                .expression_call(
                                    span,
                                    ctx.ast.expression_identifier(SPAN, "_wrapProp"),
                                    NONE,
                                    ctx.ast.vec_from_array([
                                        Argument::from(ctx.ast.expression_identifier(SPAN, ctx.ast.atom(&obj_name))),
                                        Argument::from(ctx.ast.expression_string_literal(
                                            SPAN,
                                            prop_name_str,
                                            None,
                                        )),
                                    ]),
                                    false,
                                )
                                .into(),
                        )
                    } else if crate::inlined_fn::should_wrap_in_fn_signal(expr, &iteration_vars) {
                        // Get current counter value for convert_inlined_fn and name generation
                        let counter = gen.hoisted_fn_counter;
                        if let Some(result) = crate::inlined_fn::convert_inlined_fn(
                            expr,
                            &iteration_vars,
                            counter,
                            &gen.builder,
                            gen.builder.allocator,
                        ) {
                            // Generate hoisted function name and increment counter
                            let hf_name = format!("_hf{}", counter);
                            let hf_str_name = format!("{}_str", &hf_name);
                            gen.hoisted_fn_counter += 1;

                            gen.needs_fn_signal_import = true;

                            if let Some(import_set) = gen.import_stack.last_mut() {
                                import_set.insert(Import::new(
                                    vec![ImportId::Named("_fnSignal".into())],
                                    QWIK_CORE_SOURCE,
                                ));
                            }

                            // Serialize the arrow function and push to component_hoisted_fns
                            // This will be emitted in the segment file, not the main file
                            let hoisted_fn_expr = Expression::ArrowFunctionExpression(ctx.ast.alloc(result.hoisted_fn));
                            let mut codegen = oxc_codegen::Codegen::new();
                            codegen.print_expression(&hoisted_fn_expr);
                            let hoisted_fn_code = codegen.into_source_text();
                            let hoisted_fn_code = hoisted_fn_code.replace(") => ", ")=>");
                            if let Some(hf_stack) = gen.component_hoisted_fns.last_mut() {
                                hf_stack.push((hf_name.clone(), hoisted_fn_code, result.hoisted_str.clone()));
                            }

                            // Create captures array with original variable references
                            let captures_array = ctx.ast.expression_array(
                                SPAN,
                                ctx.ast.vec_from_iter(result.captures.iter().map(|(name, _)| {
                                    ArrayExpressionElement::from(ctx.ast.expression_identifier(SPAN, ctx.ast.atom(name)))
                                })),
                            );

                            // Create _fnSignal call with hoisted identifier references
                            Some(
                                ctx.ast.expression_call(
                                    span,
                                    ctx.ast.expression_identifier(SPAN, "_fnSignal"),
                                    NONE,
                                    ctx.ast.vec_from_array([
                                        Argument::from(ctx.ast.expression_identifier(SPAN, ctx.ast.atom(&hf_name))),
                                        Argument::from(captures_array),
                                        Argument::from(ctx.ast.expression_identifier(SPAN, ctx.ast.atom(&hf_str_name))),
                                    ]),
                                    false,
                                )
                                .into(),
                            )
                        } else {
                            Some(move_expression(&gen.builder, expr).into())
                        }
                    } else {
                        Some(move_expression(&gen.builder, expr).into())
                    }
                } else {
                    Some(move_expression(&gen.builder, expr).into())
                }
                }
            }
            JSXChild::Spread(b) => {
                jsx.static_subtree = false;
                let span = (*b).span.clone();
                Some(gen.builder.array_expression_element_spread_element(
                    span,
                    move_expression(&gen.builder, &mut (*b).expression),
                ))
            }
        };
        if let Some(child) = maybe_child {
            jsx.children.push(child);
        }
    }
}
