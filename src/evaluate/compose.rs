use super::*;
mod optimize;

/// An opaque slot belonging to one [`EvaluatorComposer`].
///
/// Handles remain valid when inputs are added or constants are deduplicated.
/// A handle from another composer is rejected, even if its index happens to match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ComposerSlot {
    owner: usize,
    index: usize,
}

/// An invalid binding or incompatible function definition in a composition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeError {
    /// The evaluator's input count differs from the number of bindings.
    ArityMismatch { expected: usize, actual: usize },
    /// A binding or selected output belongs to another composer.
    ForeignSlot,
    /// Two evaluators define different functions with the same exported name.
    ConflictingFunction { symbol: Symbol },
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArityMismatch { expected, actual } => {
                write!(f, "Expected {expected} bindings, received {actual}")
            }
            Self::ForeignSlot => write!(f, "Slot belongs to another evaluator composer"),
            Self::ConflictingFunction { symbol } => {
                write!(f, "Conflicting definitions for function '{symbol}'")
            }
        }
    }
}

impl std::error::Error for ComposeError {}

/// Connect existing scalar evaluators without reconstructing symbolic expressions.
///
/// Inputs are ordered by creation. All declared inputs, including unused ones,
/// remain in the finished evaluator's signature. Applications may bind any input,
/// constant or earlier result; shared function bodies remain shared.
///
/// ```
/// use symbolica::prelude::*;
///
/// let map = parse!("x+y").evaluator(&[parse!("x"), parse!("y")]).build().unwrap();
/// let function = parse!("z^2").evaluator(&[parse!("z")]).build().unwrap();
/// let (mut composer, inputs) = EvaluatorComposer::new_with_inputs(2);
/// let mapped = composer.add(&map, &inputs).unwrap();
/// let outputs = composer.add(&function, &mapped).unwrap();
/// composer.deduplicate_constants(); // optional; requires exact-value Eq + Hash
/// let mut evaluator = composer.finish(&outputs, OptimizationSettings::default())
///     .unwrap().map_coeff(&|c| c.re.to_f64());
/// assert_eq!(evaluator.evaluate_single(&[2., 3.]), 25.);
/// ```
pub struct EvaluatorComposer<T> {
    owner: usize,
    parameters: usize,
    values: Vec<Slot>,
    instructions: InstructionList<T>,
    external_functions: Vec<ExternalFunctionContainer<T>>,
    next_label: usize,
}

impl<T: Default + Clone> Default for EvaluatorComposer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Default + Clone> EvaluatorComposer<T> {
    /// Start a program with no inputs. Use [`Self::add_input`] to declare inputs.
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT_OWNER: AtomicUsize = AtomicUsize::new(0);
        // fetch_update supports the crate's MSRV; its newer name is not available there.
        #[allow(deprecated)]
        let owner = NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("Evaluator composer identities exhausted");
        Self {
            owner,
            parameters: 0,
            values: Vec::new(),
            instructions: InstructionList {
                instructions: Vec::new(),
                constants: Vec::new(),
                unknown_constants: Vec::new(),
                dim: 1,
            },
            external_functions: Vec::new(),
            next_label: 0,
        }
    }

    /// Start a program and return its inputs in parameter order.
    pub fn new_with_inputs(count: usize) -> (Self, Vec<ComposerSlot>) {
        let mut composer = Self::new();
        let inputs = (0..count).map(|_| composer.add_input()).collect();
        (composer, inputs)
    }

    fn value(&mut self, slot: Slot) -> ComposerSlot {
        let value = ComposerSlot {
            owner: self.owner,
            index: self.values.len(),
        };
        self.values.push(slot);
        value
    }

    fn slots(&self, values: &[ComposerSlot]) -> Result<Vec<Slot>, ComposeError> {
        values
            .iter()
            .map(|value| {
                if value.owner != self.owner {
                    return Err(ComposeError::ForeignSlot);
                }
                Ok(self.values[value.index])
            })
            .collect()
    }

    /// Declare the next input. Its position is the number of inputs already created.
    pub fn add_input(&mut self) -> ComposerSlot {
        let slot = Slot::Param(self.parameters);
        self.parameters += 1;
        self.value(slot)
    }

    /// Create a literal constant, without imposing an equality policy on `T`.
    pub fn constant(&mut self, value: T) -> ComposerSlot {
        let slot = Slot::Const(self.instructions.constants.len());
        self.instructions.constants.push(value);
        self.instructions.unknown_constants.push(false);
        self.value(slot)
    }

    /// Add an evaluator bound to existing values, returning its outputs in order.
    ///
    /// The evaluator is borrowed and retains its original instructions and function
    /// bodies. Invalid bindings and conflicting function definitions leave the
    /// composer unchanged. Function bodies and their transitive callees are imported
    /// with remapped indices, preserving shared calls rather than inlining bodies.
    pub fn add(
        &mut self,
        evaluator: &ExpressionEvaluator<T>,
        inputs: &[ComposerSlot],
    ) -> Result<Vec<ComposerSlot>, ComposeError> {
        if evaluator.param_count != inputs.len() {
            return Err(ComposeError::ArityMismatch {
                expected: evaluator.param_count,
                actual: inputs.len(),
            });
        }
        let inputs = self.slots(inputs)?;
        let mut evaluator = evaluator.clone();
        evaluator.undo_stack_optimization();
        let checkpoint = (
            self.instructions.instructions.len(),
            self.instructions.constants.len(),
            self.external_functions.len(),
            self.next_label,
        );
        let outputs = evaluator.inline_vector_components(
            &inputs,
            &mut self.instructions,
            &mut self.external_functions,
            &mut self.next_label,
        );
        // Exporters identify callable functions by name. Reject incompatible
        // definitions instead of silently calling the first evaluator's body.
        let mut definitions = HashMap::default();
        let conflict = self.external_functions.iter().find_map(|function| {
            if function.constant_index.is_some() {
                return None;
            }
            definitions
                .insert(&function.export_name, function)
                .and_then(|previous| {
                    (previous != function || previous.body != function.body)
                        .then_some(function.symbol)
                })
        });
        if let Some(symbol) = conflict {
            self.instructions.instructions.truncate(checkpoint.0);
            self.instructions.constants.truncate(checkpoint.1);
            self.instructions.unknown_constants.truncate(checkpoint.1);
            self.external_functions.truncate(checkpoint.2);
            self.next_label = checkpoint.3;
            return Err(ComposeError::ConflictingFunction { symbol });
        }
        Ok(outputs.into_iter().map(|slot| self.value(slot)).collect())
    }

    /// Select/reorder outputs, prune unused work, and optimize the combined program.
    ///
    /// Dead instructions, branch regions, constants and transitive function bodies
    /// are discarded. Live branches retain lazy evaluation. Native CSE runs to a
    /// fixed point, then CPE runs up to `settings.cpe_iterations` (or to a fixed
    /// point for `None`), followed by stack optimization. Symbolic Horner search
    /// is not repeated. Literal interning is optional: call
    /// [`Self::deduplicate_constants`] first when `T` supports exact equality.
    pub fn finish(
        mut self,
        outputs: &[ComposerSlot],
        settings: OptimizationSettings,
    ) -> Result<ExpressionEvaluator<T>, ComposeError> {
        let rounds = settings.cpe_iterations;
        let mut outputs = self.slots(outputs)?;
        self.prune_unused(&mut outputs);
        Ok(ExpressionEvaluator::from_instruction_list(
            self.parameters,
            self.instructions,
            self.external_functions,
            outputs,
            settings,
            rounds,
            true,
        ))
    }
}

impl<T: Default + Clone> ExpressionEvaluator<T> {
    /// Lower scalar instruction storage shared by native vectorization/composition.
    pub(super) fn from_instruction_list(
        param_count: usize,
        instructions: InstructionList<T>,
        external_fns: Vec<ExternalFunctionContainer<T>>,
        outputs: Vec<Slot>,
        settings: OptimizationSettings,
        cpe_rounds: Option<usize>,
        eliminate_common_instructions: bool,
    ) -> Self {
        let reserved_indices = param_count + instructions.constants.len();
        let index = |slot| match slot {
            Slot::Param(index) => index,
            Slot::Const(index) => param_count + index,
            Slot::Temp(index) => reserved_indices + index,
            Slot::Out(_) => unreachable!("composition validates all slots"),
        };
        let mut lowered = Vec::with_capacity(instructions.instructions.len());
        for instruction in instructions.instructions {
            let out = reserved_indices + lowered.len();
            let instruction = match instruction {
                VectorInstruction::Add(a, b) | VectorInstruction::Mul(a, b) => {
                    let mut inputs = vec![index(a), index(b)];
                    inputs.sort_unstable();
                    if matches!(instruction, VectorInstruction::Add(..)) {
                        Instr::Add(out, inputs)
                    } else {
                        Instr::Mul(out, inputs)
                    }
                }
                VectorInstruction::Assign(a) => Instr::Add(out, vec![index(a)]),
                VectorInstruction::Pow(a, power) => Instr::Pow(out, index(a), power),
                VectorInstruction::Powf(a, b) => Instr::Powf(out, index(a), index(b)),
                VectorInstruction::BuiltinFun(symbol, a) => {
                    Instr::BuiltinFun(out, symbol, index(a))
                }
                VectorInstruction::ExternalFun(function, args) => {
                    Instr::ExternalFun(out, function, args.into_iter().map(index).collect())
                }
                VectorInstruction::IfElse(condition, label) => {
                    Instr::IfElse(index(condition), label)
                }
                VectorInstruction::Goto(label) => Instr::Goto(label),
                VectorInstruction::Label(label) => Instr::Label(label),
                VectorInstruction::Join(condition, a, b) => {
                    Instr::Join(out, index(condition), index(a), index(b))
                }
            };
            lowered.push((instruction, ComplexPhase::Any));
        }
        let mut stack = vec![T::default(); param_count];
        stack.extend(instructions.constants);
        stack.resize(reserved_indices + lowered.len(), T::default());
        let mut evaluator = Self {
            stack,
            param_count,
            reserved_indices,
            instructions: lowered,
            result_indices: outputs.into_iter().map(index).collect(),
            external_fns,
            settings,
        };
        if eliminate_common_instructions {
            evaluator.flatten_composition_operations();
            loop {
                if evaluator.settings.abort_level > 0 || evaluator.remove_common_instructions() == 0
                {
                    evaluator.settings.abort_level = 0;
                    break;
                }
            }
        }
        for _ in 0..cpe_rounds.unwrap_or(usize::MAX) {
            if (eliminate_common_instructions && evaluator.settings.abort_level > 0)
                || evaluator.remove_common_pairs() == 0
            {
                if eliminate_common_instructions {
                    evaluator.settings.abort_level = 0;
                }
                break;
            }
        }
        evaluator.optimize_stack();
        evaluator.fix_labels();
        evaluator
    }
}

#[cfg(test)]
#[path = "compose/tests.rs"]
mod composition_tests;
