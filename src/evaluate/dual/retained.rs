//! Component signatures and regression coverage for retained vectorization.

use super::*;

pub(super) type Request = (usize, Vec<(usize, usize)>);

pub(super) enum Component {
    Input(usize),
    Function { index: usize, inputs: Vec<usize> },
}

pub(super) struct Functions {
    pub components: Vec<Component>,
    pub constants: Vec<Slot>,
}

impl<T: Default + Clone> ExpressionEvaluator<T> {
    /// Keep the component's data dependencies and the original lazy branch
    /// structure. Constants become explicit function inputs, as in the native
    /// retained-body builder; unused original arguments are dropped.
    pub(super) fn retain_vector_component(&mut self, component: usize) -> Vec<usize> {
        self.result_indices = vec![self.result_indices[component]];
        self.undo_stack_optimization();
        let evaluator = self;
        let mut used = HashSet::from_iter(evaluator.result_indices.iter().copied());
        let mut keep = vec![false; evaluator.instructions.len()];
        for (index, (instruction, _)) in evaluator.instructions.iter().enumerate().rev() {
            let (output, inputs): (Option<usize>, &[usize]) = match instruction {
                Instr::Add(output, inputs)
                | Instr::Mul(output, inputs)
                | Instr::ExternalFun(output, _, inputs) => (Some(*output), inputs),
                Instr::Pow(output, input, _) | Instr::BuiltinFun(output, _, input) => {
                    (Some(*output), std::slice::from_ref(input))
                }
                Instr::Powf(output, left, right) => {
                    if used.contains(output) {
                        keep[index] = true;
                        used.extend([*left, *right]);
                    }
                    continue;
                }
                Instr::Join(output, condition, left, right) => {
                    if used.contains(output) {
                        keep[index] = true;
                        used.extend([*condition, *left, *right]);
                    }
                    continue;
                }
                Instr::IfElse(condition, _) => (None, std::slice::from_ref(condition)),
                Instr::Goto(_) | Instr::Label(_) => (None, &[]),
            };
            if output.is_none_or(|output| used.contains(&output)) {
                keep[index] = true;
                used.extend(inputs);
            }
        }
        let args = (0..evaluator.reserved_indices)
            .filter(|input| used.contains(input))
            .collect::<Vec<_>>();
        let mut slots = vec![0; evaluator.stack.len()];
        for (new, old) in args.iter().enumerate() {
            slots[*old] = new;
        }
        let mut instructions = Vec::new();
        for (index, (mut instruction, phase)) in evaluator.instructions.drain(..).enumerate() {
            if !keep[index] {
                continue;
            }
            let output = match &mut instruction {
                Instr::Add(output, inputs)
                | Instr::Mul(output, inputs)
                | Instr::ExternalFun(output, _, inputs) => {
                    for input in inputs {
                        *input = slots[*input];
                    }
                    Some(output)
                }
                Instr::Pow(output, input, _) | Instr::BuiltinFun(output, _, input) => {
                    *input = slots[*input];
                    Some(output)
                }
                Instr::Powf(output, left, right) => {
                    *left = slots[*left];
                    *right = slots[*right];
                    Some(output)
                }
                Instr::Join(output, condition, left, right) => {
                    *condition = slots[*condition];
                    *left = slots[*left];
                    *right = slots[*right];
                    Some(output)
                }
                Instr::IfElse(condition, _) => {
                    *condition = slots[*condition];
                    None
                }
                Instr::Goto(_) | Instr::Label(_) => None,
            };
            if let Some(output) = output {
                slots[*output] = args.len() + instructions.len();
                *output = slots[*output];
            }
            instructions.push((instruction, phase));
        }
        evaluator.result_indices[0] = slots[evaluator.result_indices[0]];
        evaluator.param_count = args.len();
        evaluator.reserved_indices = args.len();
        evaluator.instructions = instructions;
        evaluator.stack = vec![T::default(); args.len() + evaluator.instructions.len()];
        evaluator.optimize_stack();
        evaluator.fix_labels();
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{domains::dual::HyperDual, function, parse, symbol};

    type Number = Complex<Rational>;

    fn dualizer(shape: Vec<Vec<usize>>, zeros: Vec<(usize, usize)>) -> Dualizer<HyperDual<Number>> {
        Dualizer::new(HyperDual::new(shape), zeros)
    }

    #[test]
    fn custom_function_scope_hook_remains_supported_without_rebinding() {
        struct HookOnly(Dualizer<HyperDual<Number>>);

        impl Vectorize<Number> for HookOnly {
            fn duplicate_constants(&self) -> bool {
                self.0.duplicate_constants()
            }

            fn get_dimension(&self) -> usize {
                self.0.get_dimension()
            }

            fn map_instruction(
                &self,
                instruction: &VectorInstruction,
                instructions: &mut InstructionList<Number>,
            ) -> Vec<VectorInstruction> {
                self.0.map_instruction(instruction, instructions)
            }

            fn map_instruction_in_function(
                &self,
                instruction: &VectorInstruction,
                instructions: &mut InstructionList<Number>,
            ) -> Vec<VectorInstruction> {
                self.0
                    .map_instruction_in_function(instruction, instructions)
            }
        }

        let x = parse!("retained_custom_scope::x");
        let y = parse!("retained_custom_scope::y");
        let z = symbol!("retained_custom_scope::z");
        let f = symbol!("retained_custom_scope::f");
        let vectorizer = HookOnly(dualizer(vec![vec![0], vec![1]], vec![(0, 1)]));
        let mut evaluator = (&x + function!(f, &y))
            .evaluator(&[x, y])
            .add_function_with_options(
                f,
                vec![z],
                Atom::var(z).pow(2),
                FunctionRegistrationOptions::new().inlining(InliningPolicy::Never),
            )
            .unwrap()
            .build()
            .unwrap()
            .vectorize(&vectorizer)
            .unwrap()
            .map_coeff(&|value| value.re.to_f64());
        let mut output = [0.; 2];
        evaluator.evaluate(&[1., 0., 3., 1.], &mut output);
        assert_eq!(output, [10., 6.]);
    }

    #[test]
    fn retained_mixed_components_match_analytic_polynomial() {
        let f = symbol!("retained_vector::polynomial");
        let x = parse!("retained_vector::x");
        let y = parse!("retained_vector::y");
        let c = parse!("retained_vector::c");
        let expression = function!(f, &x, &y);
        let shape = vec![
            vec![0, 0],
            vec![1, 0],
            vec![0, 1],
            vec![1, 1],
            vec![2, 0],
            vec![0, 2],
        ];
        let zeros = (1..shape.len()).map(|component| (2, component)).collect();
        let scalar = expression
            .evaluator(&[x.clone(), y.clone(), c.clone()])
            .add_function_with_options(
                f,
                vec![
                    Indeterminate::try_from(x.clone()).unwrap(),
                    Indeterminate::try_from(y.clone()).unwrap(),
                ],
                x.pow(2) * &y + &c,
                FunctionRegistrationOptions::new().inlining(InliningPolicy::Never),
            )
            .unwrap()
            .build()
            .unwrap();
        let vector = scalar.vectorize(&dualizer(shape, zeros)).unwrap();
        assert!(
            vector
                .external_fns
                .iter()
                .any(|function| function.body.is_some())
        );
        // These integer-valued polynomial jets are exactly representable in
        // binary64. Ring evaluators do not currently support retained calls.
        let mut numeric = vector.map_coeff(&|value| value.re.to_f64());
        let input = [
            2., 1., 0., 0., 0., 0., 3., 0., 1., 0., 0., 0., 5., 0., 0., 0., 0., 0.,
        ];
        let mut output = [0.; 6];
        numeric.evaluate(&input, &mut output);
        assert_eq!(output, [17., 12., 4., 4., 3., 0.]);
    }

    #[test]
    fn retained_nested_calls_preserve_alias_scope_and_captures() {
        let f = symbol!("retained_vector_scope::outer");
        let g = symbol!("retained_vector_scope::inner");
        let global = parse!("retained_vector_scope::global");
        let local = parse!("retained_vector_scope::local");
        let x = parse!("retained_vector_scope::x");
        let y = parse!("retained_vector_scope::y");
        let alias = parse!("retained_vector_scope::alias");
        let expression = function!(f, &x) + function!(f, &y);
        let build = |policy| {
            expression
                .evaluator(&[x.clone(), y.clone(), global.clone()])
                .add_aliases([(alias.clone(), global.pow(2))])
                .unwrap()
                .add_function_with_options(
                    g,
                    vec![Indeterminate::try_from(local.clone()).unwrap()],
                    local.pow(3) + &global,
                    FunctionRegistrationOptions::new().inlining(policy),
                )
                .unwrap()
                .add_function_with_options(
                    f,
                    vec![Indeterminate::try_from(global.clone()).unwrap()],
                    function!(g, &global) + &alias,
                    FunctionRegistrationOptions::new().inlining(policy),
                )
                .unwrap()
                .build()
                .unwrap()
                .vectorize(&dualizer(vec![vec![0], vec![1], vec![2], vec![3]], vec![]))
                .unwrap()
                .map_coeff(&|value| value.re.to_f64())
        };
        let mut retained = build(InliningPolicy::Never);
        let mut inlined = build(InliningPolicy::Always);
        for input in [
            [2., 1., 0., 0., 3., 0., 1., 0., 7., 0., 0., 1.],
            [-2., 0., 1., 1., 0.5, 1., 0., 0., 3., 1., 0., 0.],
        ] {
            let mut actual = [0.; 4];
            let mut expected = [0.; 4];
            retained.evaluate(&input, &mut actual);
            inlined.evaluate(&input, &mut expected);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn retained_bodies_reuse_equal_seeds_and_separate_distinct_seeds() {
        let f = symbol!("retained_vector_cache::f");
        let x = parse!("retained_vector_cache::x");
        let y = parse!("retained_vector_cache::y");
        let scalar = (function!(f, &x) + function!(f, &y))
            .evaluator(&[x.clone(), y.clone()])
            .add_function_with_options(
                f,
                vec![Indeterminate::try_from(x.clone()).unwrap()],
                x.pow(2) + Atom::num(1),
                FunctionRegistrationOptions::new().inlining(InliningPolicy::Never),
            )
            .unwrap()
            .build()
            .unwrap();
        for (zeros, body_count, input, expected) in [
            (vec![], 2, [2., 1., 3., 1.], [15., 10.]),
            (vec![(1, 1)], 3, [2., 1., 3., 0.], [15., 4.]),
        ] {
            let vector = scalar
                .clone()
                .vectorize(&dualizer(vec![vec![0], vec![1]], zeros))
                .unwrap();
            assert_eq!(
                vector
                    .external_fns
                    .iter()
                    .filter(|function| function.body.is_some())
                    .count(),
                body_count
            );
            let mut eager = vector.map_coeff(&|value| value.re.to_f64());
            let mut output = [0.; 2];
            eager.evaluate(&input, &mut output);
            assert_eq!(output, expected);
        }
    }

    #[test]
    fn retained_nested_zero_seeds_preserve_singular_endpoints() {
        let f = symbol!("retained_vector_zero::outer");
        let g = symbol!("retained_vector_zero::inner");
        let x = parse!("retained_vector_zero::x");
        let scalar = function!(f, &x)
            .evaluator(std::slice::from_ref(&x))
            .add_function_with_options(
                g,
                vec![Indeterminate::try_from(x.clone()).unwrap()],
                Symbol::SQRT.call_args([x.clone()]),
                FunctionRegistrationOptions::new().inlining(InliningPolicy::Never),
            )
            .unwrap()
            .add_function_with_options(
                f,
                vec![Indeterminate::try_from(x.clone()).unwrap()],
                function!(g, &x) + &x,
                FunctionRegistrationOptions::new().inlining(InliningPolicy::Never),
            )
            .unwrap()
            .build()
            .unwrap();
        let vector = scalar
            .vectorize(&dualizer(
                vec![vec![0], vec![1], vec![2]],
                vec![(0, 1), (0, 2)],
            ))
            .unwrap();
        let mut eager =
            vector.map_coeff(&|value| Complex::new(value.re.to_f64(), value.im.to_f64()));
        let mut output = [Complex::default(); 3];
        eager.evaluate(&[Complex::default(); 3], &mut output);
        assert_eq!(output, [Complex::default(); 3]);
    }

    #[test]
    fn zero_seeds_propagate_through_intermediate_arithmetic() {
        let q = crate::symbol!("retained_vector_zero_temporary::q");
        let x = crate::symbol!("retained_vector_zero_temporary::x");
        let function = crate::symbol!("retained_vector_zero_temporary::function");
        for inlining in [InliningPolicy::Always, InliningPolicy::Never] {
            let vector = crate::function!(function, q, x)
                .evaluator(&[Atom::var(q), Atom::var(x)])
                .add_function_with_options(
                    function,
                    vec![q, x],
                    Symbol::SQRT.call_args([Atom::var(q).pow(2) + Atom::var(q)]) * Atom::var(x),
                    FunctionRegistrationOptions::new().inlining(inlining),
                )
                .unwrap()
                .build()
                .unwrap()
                .vectorize(&dualizer(
                    vec![vec![0], vec![1], vec![2]],
                    vec![(0, 1), (0, 2)],
                ))
                .unwrap();
            let mut numeric =
                vector.map_coeff(&|value| Complex::new(value.re.to_f64(), value.im.to_f64()));
            let mut output = [Complex::default(); 3];
            numeric.evaluate(
                &[0., 0., 0., 2., 1., 0.].map(|value| Complex::new(value, 0.)),
                &mut output,
            );
            assert_eq!(output, [Complex::default(); 3], "{inlining:?}");
        }
    }

    #[test]
    fn nested_zero_seeds_keep_guards_and_higher_jets() {
        let q = crate::symbol!("retained_vector_guarded_zero::q");
        let x = crate::symbol!("retained_vector_guarded_zero::x");
        let inner = crate::symbol!("retained_vector_guarded_zero::inner");
        let outer = crate::symbol!("retained_vector_guarded_zero::outer");
        let options = FunctionRegistrationOptions::new().inlining(InliningPolicy::Never);
        let vector = crate::function!(outer, q, x)
            .evaluator(&[Atom::var(q), Atom::var(x)])
            .add_function_with_options(
                inner,
                vec![q, x],
                Symbol::SQRT.call_args([Atom::var(q).pow(2) + Atom::var(q)]) * Atom::var(x),
                options.clone(),
            )
            .unwrap()
            .add_function_with_options(
                outer,
                vec![q, x],
                Symbol::IF.call_args([
                    Atom::var(x),
                    crate::function!(inner, q, x) + Atom::var(x).pow(2),
                    Atom::var(q).pow(-1),
                ]),
                options,
            )
            .unwrap()
            .build()
            .unwrap()
            .vectorize(&dualizer(
                vec![vec![0], vec![1], vec![2], vec![3]],
                vec![(0, 1), (0, 2), (0, 3)],
            ))
            .unwrap();
        let input = [0., 0., 0., 0., 2., 1., 0., 0.];
        let mut output = [0.; 4];
        vector
            .clone()
            .map_coeff(&|value| value.re.to_f64())
            .evaluate(&input, &mut output);
        assert_eq!(output, [4., 4., 1., 0.]);
        #[cfg(feature = "native_code_generation")]
        for direct in [false, true] {
            let mut jit = vector
                .jit_compile::<f64>(
                    JITCompilationSettings::new()
                        .optimization_level(2)
                        .direct_translation(direct),
                )
                .unwrap();
            jit.evaluate(&input, &mut output);
            assert_eq!(output, [4., 4., 1., 0.]);
        }
    }

    #[test]
    fn inferred_zero_seeds_do_not_cancel_invalid_scalar_values() {
        let a = crate::symbol!("retained_vector_invalid::a");
        let x = crate::symbol!("retained_vector_invalid::x");
        let function = crate::symbol!("retained_vector_invalid::function");
        let scalar = crate::function!(function, 0, x)
            .evaluator(&[Atom::var(x)])
            .add_function_with_options(
                function,
                vec![a, x],
                Atom::var(a) / Atom::var(x),
                FunctionRegistrationOptions::new().inlining(InliningPolicy::Never),
            )
            .unwrap()
            .build()
            .unwrap();
        let mut ordinary = scalar
            .clone()
            .map_coeff(&|value| Complex::new(value.re.to_f64(), value.im.to_f64()));
        let original = ordinary.evaluate_single(&[Complex::default()]);
        assert!(!original.re.is_finite() || !original.im.is_finite());
        let vector = scalar
            .vectorize(&dualizer(vec![vec![0], vec![1], vec![2]], vec![]))
            .unwrap();
        let mut numeric =
            vector.map_coeff(&|value| Complex::new(value.re.to_f64(), value.im.to_f64()));
        let mut output = [Complex::default(); 3];
        numeric.evaluate(
            &[0., 1., 0.].map(|value| Complex::new(value, 0.)),
            &mut output,
        );
        assert!(
            !output[0].re.is_finite() || !output[0].im.is_finite(),
            "derivative specialization must not make an invalid primal finite"
        );
        numeric.evaluate(
            &[2., 1., 0.].map(|value| Complex::new(value, 0.)),
            &mut output,
        );
        assert_eq!(output, [Complex::default(); 3]);
    }

    #[test]
    fn retained_identity_and_constant_values_keep_their_native_domain() {
        let a = crate::symbol!("retained_vector_identity_domain::a");
        let x = crate::symbol!("retained_vector_identity_domain::x");
        let f = crate::symbol!("retained_vector_identity_domain::f");
        for body in [Atom::var(a), Atom::Zero] {
            let scalar = (crate::function!(f, 0) / Atom::var(x))
                .evaluator(&[Atom::var(x)])
                .add_function_with_options(
                    f,
                    vec![a],
                    body,
                    FunctionRegistrationOptions::new().inlining(InliningPolicy::Never),
                )
                .unwrap()
                .build()
                .unwrap();
            let mut original = scalar
                .clone()
                .map_coeff(&|value| Complex::new(value.re.to_f64(), value.im.to_f64()));
            let reference = original.evaluate_single(&[Complex::default()]);
            assert!(!reference.re.is_finite() || !reference.im.is_finite());
            let vector = scalar
                .vectorize(&dualizer(vec![vec![0], vec![1]], vec![]))
                .unwrap();
            let mut numeric =
                vector.map_coeff(&|value| Complex::new(value.re.to_f64(), value.im.to_f64()));
            let mut output = [Complex::default(); 2];
            numeric.evaluate(&[0., 1.].map(|value| Complex::new(value, 0.)), &mut output);
            assert!(
                !output[0].re.is_finite() || !output[0].im.is_finite(),
                "retained scalar output must not become an algebraically cancelling literal"
            );
            numeric.evaluate(&[2., 1.].map(|value| Complex::new(value, 0.)), &mut output);
            assert_eq!(output, [Complex::default(); 2]);
        }
    }

    #[test]
    fn retained_vectorization_keeps_inactive_poles_lazy() {
        let f = symbol!("retained_vector_lazy::f");
        let x = parse!("retained_vector_lazy::x");
        let condition = parse!("retained_vector_lazy::condition");
        let vector = function!(f, &x)
            .evaluator(&[x.clone(), condition.clone()])
            .add_function_with_options(
                f,
                vec![Indeterminate::try_from(x.clone()).unwrap()],
                Symbol::IF.call_args([condition.clone(), x.pow(2), x.pow(-1)]),
                FunctionRegistrationOptions::new().inlining(InliningPolicy::Never),
            )
            .unwrap()
            .build()
            .unwrap()
            .vectorize(&dualizer(
                vec![vec![0], vec![1], vec![2]],
                vec![(1, 1), (1, 2)],
            ))
            .unwrap();
        let mut eager = vector.clone().map_coeff(&|value| value.re.to_f64());
        for (input, expected) in [
            ([0., 1., 0., 1., 0., 0.], [0., 0., 1.]),
            ([2., 1., 0., 0., 0., 0.], [0.5, -0.25, 0.125]),
        ] {
            let mut output = [0.; 3];
            eager.evaluate(&input, &mut output);
            assert_eq!(output, expected);
            #[cfg(feature = "native_code_generation")]
            for direct in [false, true] {
                let mut jit = vector
                    .jit_compile::<f64>(
                        JITCompilationSettings::new()
                            .optimization_level(2)
                            .direct_translation(direct),
                    )
                    .unwrap();
                jit.evaluate(&input, &mut output);
                assert_eq!(output, expected);
            }
        }
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_vec(&eager).unwrap();
            let mut restored: ExpressionEvaluator<f64> = serde_json::from_slice(&json).unwrap();
            let mut output = [0.; 3];
            restored.evaluate(&[2., 1., 0., 0., 0., 0.], &mut output);
            assert_eq!(output, [0.5, -0.25, 0.125]);
        }
        #[cfg(feature = "bincode")]
        {
            let bytes = bincode::encode_to_vec(&eager, bincode::config::standard()).unwrap();
            let (mut restored, _): (ExpressionEvaluator<f64>, _) =
                bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
            let mut output = [0.; 3];
            restored.evaluate(&[2., 1., 0., 0., 0., 0.], &mut output);
            assert_eq!(output, [0.5, -0.25, 0.125]);
        }
    }
}
