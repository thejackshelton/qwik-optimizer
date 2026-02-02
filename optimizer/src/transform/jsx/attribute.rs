use oxc_allocator::CloneIn;
use oxc_ast::ast::*;
use oxc_ast::NONE;
use oxc_ast_visit::Visit;
use oxc_span::{GetSpan, SPAN};
use oxc_traverse::TraverseCtx;

use crate::collector::IdentCollector;
use crate::component::{Import, QrlComponent, QrlType, SegmentData, MARKER_SUFFIX};
use crate::is_const::is_const_expr;
use crate::transform::generator::{IdentType, IdPlusType, TransformGenerator};
use crate::transform::qrl as qrl_module;

use super::bind::{create_bind_handler, is_bind_directive, merge_event_handlers};
use super::event::jsx_event_to_html_attribute;
use super::{get_jsx_attribute_full_name, move_expression, _GET_VAR_PROPS};

/// Creates a PropertyKey for JSX attributes.
/// Uses StringLiteral for keys containing colons (q:p, on:click, etc.) to match qwik-core.
/// Uses StaticIdentifier for standard keys (class, id, etc.).
fn create_property_key<'a>(
    builder: &oxc_ast::AstBuilder<'a>,
    span: oxc_span::Span,
    name: &str,
) -> PropertyKey<'a> {
    if name.contains(':') {
        PropertyKey::StringLiteral(builder.alloc(
            builder.string_literal(span, builder.atom(name), None)
        ))
    } else {
        builder.property_key_static_identifier(span, builder.atom(name))
    }
}

pub fn enter_jsx_attribute<'a>(
    gen: &mut TransformGenerator<'a>,
    node: &mut JSXAttribute<'a>,
    ctx: &mut TraverseCtx<'a, ()>,
) {
    if gen.options.transpile_jsx {
        gen.expr_is_const_stack.push(
            gen.jsx_stack
                .last()
                .is_some_and(|jsx| !jsx.should_runtime_sort),
        );
    }
    gen.ascend();
    gen.debug("ENTER: JSXAttribute", ctx);
    let segment_name = match &node.name {
        JSXAttributeName::Identifier(id) => id.name.to_string(),
        JSXAttributeName::NamespacedName(ns) => ns.name.name.to_string(),
    };
    let segment = gen.new_segment(segment_name);
    gen.segment_stack.push(segment);

    let attr_name = get_jsx_attribute_full_name(&node.name);

    let is_native = gen.jsx_element_is_native.last().copied().unwrap_or(false);
    let stack_ctxt_name = if is_native {
        if let Some(html_attr) = jsx_event_to_html_attribute(&attr_name) {
            html_attr.to_string()
        } else {
            attr_name.clone()
        }
    } else {
        attr_name.clone()
    };
    gen.stack_ctxt.push(stack_ctxt_name);

    let is_native = gen.jsx_element_is_native.last().copied().unwrap_or(false);
    if is_native {
        if let Some(is_checked) = is_bind_directive(&attr_name) {
            if let Some(JSXAttributeValue::ExpressionContainer(container)) = &node.value {
                if let Some(expr) = container.expression.as_expression() {
                    gen.pending_bind_directives.push((
                        is_checked,
                        expr.clone_in(ctx.ast.allocator),
                    ));
                    if is_checked {
                        gen.needs_chk_import = true;
                    } else {
                        gen.needs_val_import = true;
                    }
                    gen.needs_inlined_qrl_import = true;
                }
            }
        }
    }

    if attr_name.ends_with(MARKER_SUFFIX) {
        if let Some(JSXAttributeValue::ExpressionContainer(container)) = &node.value {
            if let Some(expr) = container.expression.as_expression() {
                let is_fn = matches!(
                    expr,
                    Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)
                );

                if is_fn {
                    gen.import_stack.push(std::collections::BTreeSet::new());
                }
            }
        }
    }
}

pub fn exit_jsx_attribute<'a>(
    gen: &mut TransformGenerator<'a>,
    node: &mut JSXAttribute<'a>,
    ctx: &mut TraverseCtx<'a, ()>,
) {
    let attr_name = get_jsx_attribute_full_name(&node.name);
    let is_native = gen.jsx_element_is_native.last().copied().unwrap_or(false);

    if is_native && gen.options.transpile_jsx {
        if let Some(is_checked) = is_bind_directive(&attr_name) {
            if let Some(JSXAttributeValue::ExpressionContainer(container)) = &node.value {
                if let Some(expr) = container.expression.as_expression() {
                    let signal_expr = expr.clone_in(ctx.ast.allocator);
                    let prop_name = if is_checked { "checked" } else { "value" };

                    let bind_handler = create_bind_handler(
                        &ctx.ast,
                        is_checked,
                        signal_expr.clone_in(ctx.ast.allocator),
                    );

                    gen.expr_is_const_stack.pop();

                    if let Some(jsx) = gen.jsx_stack.last_mut() {
                        let prop_name_atom = gen.builder.atom(prop_name);
                        jsx.const_props.push(gen.builder.object_property_kind_object_property(
                            node.span,
                            PropertyKind::Init,
                            gen.builder.property_key_static_identifier(SPAN, prop_name_atom),
                            signal_expr,
                            false,
                            false,
                            false,
                        ));

                        let existing_on_input_idx = jsx.const_props.iter().position(|prop| {
                            if let ObjectPropertyKind::ObjectProperty(obj_prop) = prop {
                                if let PropertyKey::StaticIdentifier(id) = &obj_prop.key {
                                    return id.name == "on:input";
                                }
                            }
                            false
                        });

                        if let Some(idx) = existing_on_input_idx {
                            if let ObjectPropertyKind::ObjectProperty(obj_prop) = &jsx.const_props[idx]
                            {
                                let existing_handler = obj_prop.value.clone_in(ctx.ast.allocator);
                                let merged = merge_event_handlers(
                                    &ctx.ast,
                                    existing_handler,
                                    bind_handler,
                                );

                                jsx.const_props[idx] =
                                    gen.builder.object_property_kind_object_property(
                                        node.span,
                                        PropertyKind::Init,
                                        create_property_key(&gen.builder, SPAN, "on:input"),
                                        merged,
                                        false,
                                        false,
                                        false,
                                    );
                            }
                        } else {
                            jsx.const_props.push(gen.builder.object_property_kind_object_property(
                                node.span,
                                PropertyKind::Init,
                                create_property_key(&gen.builder, SPAN, "on:input"),
                                bind_handler,
                                false,
                                false,
                                false,
                            ));
                        }
                    }

                    gen.segment_stack.pop();
                    gen.stack_ctxt.pop();
                    gen.debug("EXIT: JSXAttribute (bind directive)", ctx);
                    gen.descend();
                    return;
                }
            }
        }
    }

    if attr_name.ends_with(MARKER_SUFFIX) && is_native {
        if let Some(html_attr) = jsx_event_to_html_attribute(&attr_name) {
            let new_name = gen.builder.atom(&html_attr);
            node.name = JSXAttributeName::Identifier(gen.builder.alloc(JSXIdentifier {
                span: node.name.span(),
                name: new_name,
            }));
        }
    }

    if attr_name.ends_with(MARKER_SUFFIX) {
        if let Some(JSXAttributeValue::ExpressionContainer(container)) = &mut node.value {
            if let Some(expr) = container.expression.as_expression() {
                let is_fn = matches!(
                    expr,
                    Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)
                );

                if is_fn {
                    let descendent_idents = {
                        let mut collector = IdentCollector::new();
                        collector.visit_expression(expr);
                        collector.get_words()
                    };

                    let all_decl: Vec<IdPlusType> = gen
                        .decl_stack
                        .iter()
                        .flat_map(|v| v.iter())
                        .cloned()
                        .collect();
                    let (decl_collect, _): (Vec<_>, Vec<_>) = all_decl
                        .into_iter()
                        .partition(|(_, t)| matches!(t, IdentType::Var(_)));
                    let (scoped_idents, _) =
                        qrl_module::compute_scoped_idents(&descendent_idents, &decl_collect);

                    let imports = gen.import_stack.pop().unwrap_or_default();
                    let imports_vec: Vec<_> = imports.iter().cloned().collect();
                    let imported_names = qrl_module::collect_imported_names(&imports_vec);
                    let scoped_idents = qrl_module::filter_imported_from_scoped(scoped_idents, &imported_names);

                    let referenced_exports = qrl_module::collect_referenced_exports(
                        &descendent_idents,
                        &imported_names,
                        &scoped_idents,
                        &gen.export_by_name,
                    );

                    // Partition scoped_idents based on loop iteration variables
                    // Innermost iteration var becomes a param, outer vars stay as captures
                    let (iteration_params, lexical_captures): (Vec<_>, Vec<_>) = if gen.loop_depth > 0 {
                        let current_iteration_vars = gen.iteration_var_stack.last().cloned().unwrap_or_default();
                        scoped_idents
                            .iter()
                            .cloned()
                            .partition(|(name, _)| current_iteration_vars.iter().any(|(iv_name, _)| iv_name == name))
                    } else {
                        (vec![], scoped_idents.clone())
                    };

                    let display_name = gen.current_display_name();

                    // When inside a loop, extract the handler to a separate segment file
                    // and hoist the QRL declaration to the component function level
                    if gen.loop_depth > 0 {
                        let ctx_name = attr_name.clone();
                        let hash = qrl_module::compute_hash(
                            &gen.source_info.rel_path,
                            &display_name,
                            gen.scope.as_deref(),
                        );

                        let segment_data = SegmentData::new_with_iteration_params(
                            &ctx_name,
                            display_name.clone(),
                            hash.clone(),
                            gen.source_info.rel_path.clone(),
                            lexical_captures.clone(),
                            descendent_idents.clone(),
                            None,
                            referenced_exports.clone(),
                            iteration_params.clone(),
                        );

                        let handler_expr = expr.clone_in(ctx.ast.allocator);
                        let comp = QrlComponent::from_expression_with_qrl_type(
                            handler_expr,
                            imports_vec.clone(),
                            &gen.segment_stack,
                            &gen.scope,
                            &gen.options,
                            gen.source_info,
                            Some(segment_data),
                            None,
                            QrlType::Qrl,  // Use plain qrl() for extracted handlers
                        );

                        // Create QRL that imports from the extracted segment file
                        let qrl = comp.qrl.clone();
                        gen.components.push(comp);

                        let call_expr = qrl.into_call_expression(
                            ctx,
                            &mut gen.symbol_by_name,
                            &mut gen.import_by_symbol,
                            gen.hoisted_imports_stack.last_mut().expect("hoisted_imports_stack should not be empty"),
                        );

                        // Hoist the QRL declaration to the component function level
                        // Use the same display_name that's used in the qrl() call
                        let const_name = qrl.display_name.clone();

                        // Serialize the QRL call expression to code for hoisting
                        let qrl_call_expr = Expression::CallExpression(ctx.ast.alloc(call_expr));
                        let mut codegen = oxc_codegen::Codegen::new();
                        codegen.print_expression(&qrl_call_expr);
                        let qrl_call_code = codegen.into_source_text();
                        // Apply qwik-core formatting
                        let qrl_call_code = qrl_call_code.replace("/* @__PURE__ */", "/*#__PURE__*/");
                        let qrl_call_code = qrl_call_code.replace(") => ", ")=>");

                        // Add to hoisted QRLs stack (will be injected into function body)
                        gen.add_hoisted_qrl(const_name.clone(), qrl_call_code);

                        // Replace inline QRL call with identifier reference
                        let ident_ref = ctx.ast.expression_identifier(SPAN, ctx.ast.atom(&const_name));
                        container.expression = JSXExpression::from(ident_ref);

                        // Add q:p (single) or q:ps (multiple) prop if this handler uses iteration variables
                        // Only add for native elements, and only once per element
                        if !iteration_params.is_empty() && is_native && gen.options.transpile_jsx {
                            if let Some(jsx) = gen.jsx_stack.last_mut() {
                                if !jsx.added_iter_var_prop {
                                    let (prop_name, value): (&str, Expression<'a>) =
                                        if iteration_params.len() == 1 {
                                            // Single parameter: use q:p with identifier
                                            let ident_name = &iteration_params[0].0;
                                            (
                                                "q:p",
                                                ctx.ast.expression_identifier(
                                                    SPAN,
                                                    ctx.ast.atom(ident_name),
                                                ),
                                            )
                                        } else {
                                            // Multiple parameters: use q:ps with array
                                            let elements = ctx.ast.vec_from_iter(
                                                iteration_params.iter().map(|(name, _)| {
                                                    ArrayExpressionElement::from(
                                                        ctx.ast.expression_identifier(
                                                            SPAN,
                                                            ctx.ast.atom(name),
                                                        ),
                                                    )
                                                }),
                                            );
                                            (
                                                "q:ps",
                                                ctx.ast.expression_array(SPAN, elements),
                                            )
                                        };

                                    jsx.var_props.push(
                                        gen.builder.object_property_kind_object_property(
                                            SPAN,
                                            PropertyKind::Init,
                                            create_property_key(&gen.builder, SPAN, prop_name),
                                            value,
                                            false,
                                            false,
                                            false,
                                        ),
                                    );
                                    jsx.added_iter_var_prop = true;
                                }
                            }
                        }
                    } else if gen.is_inline() {
                        // Inline/Hoist strategy: generate inlinedQrl, keep code in main file
                        let hash = qrl_module::compute_hash(
                            &gen.source_info.rel_path,
                            &display_name,
                            gen.scope.as_deref(),
                        );
                        let symbol_name = format!("{}_{}", display_name, hash);

                        // Transform the function expression to add useLexicalScope if captures exist
                        let transformed_expr = if !lexical_captures.is_empty() {
                            use crate::code_move::transform_function_expr;
                            let expr_cloned = expr.clone_in(ctx.ast.allocator);
                            transform_function_expr(expr_cloned, &lexical_captures, ctx.ast.allocator)
                        } else {
                            expr.clone_in(ctx.ast.allocator)
                        };

                        // Build inlinedQrl(expr, "symbol_name") or inlinedQrl(expr, "symbol_name", [captures])
                        let mut args: oxc_allocator::Vec<'a, Argument<'a>> = if lexical_captures.is_empty() {
                            ctx.ast.vec_with_capacity(2)
                        } else {
                            ctx.ast.vec_with_capacity(3)
                        };

                        args.push(Argument::from(transformed_expr));
                        args.push(Argument::from(ctx.ast.expression_string_literal(
                            SPAN,
                            ctx.ast.atom(&symbol_name),
                            None,
                        )));

                        if !lexical_captures.is_empty() {
                            let mut elements: oxc_allocator::Vec<'a, ArrayExpressionElement<'a>> =
                                ctx.ast.vec_with_capacity(lexical_captures.len());
                            for (name, _scope_id) in &lexical_captures {
                                let ident_ref = ctx.ast.expression_identifier(SPAN, ctx.ast.atom(name.as_str()));
                                elements.push(ArrayExpressionElement::from(ident_ref));
                            }
                            let captures_array = ctx.ast.expression_array(SPAN, elements);
                            args.push(Argument::from(captures_array));

                            // Add useLexicalScope import
                            if let Some(import_set) = gen.import_stack.last_mut() {
                                import_set.insert(Import::use_lexical_scope());
                            }
                        }

                        // Create inlinedQrl call with PURE annotation
                        let inlined_qrl_call = ctx.ast.call_expression_with_pure(
                            SPAN,
                            ctx.ast.expression_identifier(SPAN, "inlinedQrl"),
                            NONE,
                            args,
                            false,
                            true, // pure: true
                        );

                        gen.needs_inlined_qrl_import = true;

                        container.expression =
                            JSXExpression::from(Expression::CallExpression(ctx.ast.alloc(inlined_qrl_call)));
                    } else {
                        // Segment strategy: generate qrl with separate segment files
                        // Must create a QrlComponent to properly generate the segment file path
                        let ctx_name = attr_name.clone();
                        let hash = qrl_module::compute_hash(
                            &gen.source_info.rel_path,
                            &display_name,
                            gen.scope.as_deref(),
                        );

                        let segment_data = SegmentData::new_with_iteration_params(
                            &ctx_name,
                            display_name.clone(),
                            hash.clone(),
                            gen.source_info.rel_path.clone(),
                            lexical_captures.clone(),
                            descendent_idents.clone(),
                            None,
                            referenced_exports.clone(),
                            iteration_params.clone(),
                        );

                        // Get entry based on entry_strategy (e.g., "entry_segments" for Single strategy)
                        let entry = gen.get_entry_for_segment(&segment_data);

                        let handler_expr = expr.clone_in(ctx.ast.allocator);
                        let comp = QrlComponent::from_expression_with_qrl_type(
                            handler_expr,
                            imports_vec.clone(),
                            &gen.segment_stack,
                            &gen.scope,
                            &gen.options,
                            gen.source_info,
                            Some(segment_data),
                            entry,
                            QrlType::Qrl,
                        );

                        let qrl = comp.qrl.clone();
                        gen.components.push(comp);

                        let call_expr = qrl.into_call_expression(
                            ctx,
                            &mut gen.symbol_by_name,
                            &mut gen.import_by_symbol,
                            gen.hoisted_imports_stack.last_mut().expect("hoisted_imports_stack should not be empty"),
                        );

                        container.expression =
                            JSXExpression::from(Expression::CallExpression(ctx.ast.alloc(call_expr)));

                        if let Some(import_set) = gen.import_stack.last_mut() {
                            import_set.insert(Import::qrl());
                        }
                    }
                }
            }
        }
    }

    if gen.options.transpile_jsx {
        let prop_wrap_key: Option<String> =
            if let Some(JSXAttributeValue::ExpressionContainer(container)) = &node.value {
                if let Some(expr) = container.expression.as_expression() {
                    gen.should_wrap_prop(expr).map(|(_, key)| key)
                } else {
                    None
                }
            } else {
                None
            };

        let needs_signal_wrap: bool = if prop_wrap_key.is_none() {
            if let Some(JSXAttributeValue::ExpressionContainer(container)) = &node.value {
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

        let stack_is_const = gen.expr_is_const_stack.pop().unwrap_or_default();
        let is_const = if let Some(JSXAttributeValue::ExpressionContainer(container)) = &node.value
        {
            if let Some(value_expr) = container.expression.as_expression() {
                if stack_is_const {
                    let import_names = gen.get_imported_names();
                    is_const_expr(value_expr, &import_names, &gen.decl_stack)
                } else {
                    false
                }
            } else {
                stack_is_const
            }
        } else {
            stack_is_const
        };

        if let Some(jsx) = gen.jsx_stack.last_mut() {
            let expr: Expression<'a> = {
                let v = &mut node.value;
                match v {
                    None => gen.builder.expression_boolean_literal(node.span, true),
                    Some(JSXAttributeValue::Element(_)) => gen.replace_expr.take().unwrap(),
                    Some(JSXAttributeValue::Fragment(_)) => gen.replace_expr.take().unwrap(),
                    Some(JSXAttributeValue::StringLiteral(b)) => gen
                        .builder
                        .expression_string_literal((*b).span, (*b).value, Some((*b).value)),
                    Some(JSXAttributeValue::ExpressionContainer(b)) => {
                        let inner_expr = (*b).expression.to_expression_mut();
                        let span = inner_expr.span();

                        if let Some(prop_key) = &prop_wrap_key {
                            gen.needs_wrap_prop_import = true;
                            let prop_key_str: &'a str = ctx.ast.allocator.alloc_str(prop_key);
                            ctx.ast.expression_call(
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
                        } else if needs_signal_wrap {
                            gen.needs_wrap_prop_import = true;
                            if let Expression::StaticMemberExpression(static_member) = inner_expr {
                                let signal_expr = static_member.object.clone_in(ctx.ast.allocator);
                                ctx.ast.expression_call(
                                    span,
                                    ctx.ast.expression_identifier(SPAN, "_wrapProp"),
                                    NONE,
                                    ctx.ast.vec1(Argument::from(signal_expr)),
                                    false,
                                )
                            } else {
                                move_expression(&gen.builder, inner_expr)
                            }
                        } else if !is_const {
                            // Get scoped identifiers for _fnSignal detection
                            // Inside loops: use iteration_var_stack
                            // Outside loops: collect all scoped idents from decl_stack
                            let scoped_idents: Vec<(String, oxc_semantic::ScopeId)> = if gen.loop_depth > 0 {
                                gen.iteration_var_stack.last().cloned().unwrap_or_default()
                            } else {
                                // Collect all declared variables that could be captured
                                gen.decl_stack
                                    .iter()
                                    .flat_map(|v| v.iter())
                                    .filter_map(|((name, _), ident_type)| {
                                        if let IdentType::Var(_) = ident_type {
                                            Some((name.clone(), oxc_semantic::ScopeId::new(0)))
                                        } else {
                                            None
                                        }
                                    })
                                    .collect()
                            };
                            if crate::inlined_fn::should_wrap_in_fn_signal(inner_expr, &scoped_idents) {
                                // Get current counter value for convert_inlined_fn and name generation
                                let counter = gen.hoisted_fn_counter;
                                if let Some(result) = crate::inlined_fn::convert_inlined_fn(
                                    inner_expr,
                                    &scoped_idents,
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
                                            vec![crate::component::ImportId::Named("_fnSignal".into())],
                                            crate::component::QWIK_CORE_SOURCE,
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
                                } else {
                                    move_expression(&gen.builder, inner_expr)
                                }
                            } else {
                                move_expression(&gen.builder, inner_expr)
                            }
                        } else {
                            move_expression(&gen.builder, inner_expr)
                        }
                    }
                }
            };
            if node.is_key() {
                jsx.key_prop = Some(expr);
            } else {
                let prop_name = get_jsx_attribute_full_name(&node.name);

                if prop_name == "on:input" {
                    let existing_on_input_idx = jsx.const_props.iter().position(|prop| {
                        if let ObjectPropertyKind::ObjectProperty(obj_prop) = prop {
                            if let PropertyKey::StaticIdentifier(id) = &obj_prop.key {
                                return id.name == "on:input";
                            }
                        }
                        false
                    });

                    if let Some(idx) = existing_on_input_idx {
                        if let ObjectPropertyKind::ObjectProperty(obj_prop) = &jsx.const_props[idx] {
                            let existing_handler = obj_prop.value.clone_in(ctx.ast.allocator);
                            let merged = merge_event_handlers(
                                &ctx.ast,
                                expr,
                                existing_handler,
                            );

                            jsx.const_props[idx] = gen.builder.object_property_kind_object_property(
                                node.span,
                                PropertyKind::Init,
                                create_property_key(&gen.builder, SPAN, &prop_name),
                                merged,
                                false,
                                false,
                                false,
                            );
                        }
                    } else {
                        let props = if is_const {
                            &mut jsx.const_props
                        } else {
                            &mut jsx.var_props
                        };
                        props.push(gen.builder.object_property_kind_object_property(
                            node.span,
                            PropertyKind::Init,
                            create_property_key(&gen.builder, node.name.span(), &prop_name),
                            expr,
                            false,
                            false,
                            false,
                        ));
                    }
                } else {
                    let props = if is_const {
                        &mut jsx.const_props
                    } else {
                        &mut jsx.var_props
                    };
                    props.push(gen.builder.object_property_kind_object_property(
                        node.span,
                        PropertyKind::Init,
                        create_property_key(&gen.builder, node.name.span(), &prop_name),
                        expr,
                        false,
                        false,
                        false,
                    ));
                }
            }
        }
    }
    let _popped = gen.segment_stack.pop();
    gen.stack_ctxt.pop();
    gen.debug("EXIT: JSXAttribute", ctx);
    gen.descend();
}

pub fn exit_jsx_spread_attribute<'a>(
    gen: &mut TransformGenerator<'a>,
    node: &mut JSXSpreadAttribute<'a>,
    _ctx: &mut TraverseCtx<'a, ()>,
) {
    if !gen.options.transpile_jsx {
        return;
    }
    if let Some(jsx) = gen.jsx_stack.last_mut() {
        let range = 0..jsx.const_props.len();
        jsx.const_props
            .drain(range)
            .for_each(|p| jsx.var_props.push(p));
        jsx.should_runtime_sort = true;
        jsx.static_subtree = false;
        jsx.static_listeners = false;

        let spread_arg = move_expression(&gen.builder, &mut node.argument);
        jsx.spread_expr = Some(spread_arg.clone_in(gen.builder.allocator));

        let get_var_props_call = gen.builder.expression_call(
            node.span(),
            gen.builder.expression_identifier(node.span(), _GET_VAR_PROPS),
            NONE,
            gen.builder.vec1(Argument::from(spread_arg)),
            false,
        );
        jsx.var_props
            .push(gen.builder.object_property_kind_spread_property(
                node.span(),
                get_var_props_call,
            ))
    }
}

pub fn exit_jsx_attribute_value<'a>(
    gen: &mut TransformGenerator<'a>,
    node: &mut JSXAttributeValue<'a>,
    ctx: &mut TraverseCtx<'a, ()>,
) {
    if let JSXAttributeValue::ExpressionContainer(container) = node {
        let qrl = gen.qrl_stack.pop();

        if let Some(qrl) = qrl {
            container.expression = qrl.into_jsx_expression(
                ctx,
                &mut gen.symbol_by_name,
                &mut gen.import_by_symbol,
                gen.hoisted_imports_stack.last_mut().expect("hoisted_imports_stack should not be empty"),
            )
        }
    }
}
