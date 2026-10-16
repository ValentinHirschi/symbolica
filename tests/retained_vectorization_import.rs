#![cfg(feature = "bincode")]

use std::{fs, process::Command};

use symbolica::{
    domains::dual::HyperDual,
    evaluate::{FunctionRegistrationOptions, InliningPolicy},
    prelude::*,
};

type Number = Complex<Rational>;

fn evaluator(power: i64) -> ExpressionEvaluator<Number> {
    let x = symbol!("retained_import::x");
    let f = symbol!(format!("retained_import::power_{power}"));
    function!(f, x)
        .evaluator(&[Atom::var(x)])
        .add_function_with_options(
            f,
            vec![x],
            Atom::var(x).pow(power),
            FunctionRegistrationOptions::new().inlining(InliningPolicy::Never),
        )
        .unwrap()
        .build()
        .unwrap()
        .vectorize(&Dualizer::new(
            HyperDual::<Number>::new(vec![vec![0], vec![1]]),
            vec![],
        ))
        .unwrap()
}

#[test]
fn imported_vectorized_bodies_keep_identity_when_merged_with_fresh_bodies() {
    const ROLE: &str = "SYMBOLICA_RETAINED_IMPORT_ROLE";
    const PAYLOAD: &str = "SYMBOLICA_RETAINED_IMPORT_PAYLOAD";
    let Ok(role) = std::env::var(ROLE) else {
        let path = std::env::temp_dir().join(format!(
            "symbolica-retained-vector-import-{}",
            std::process::id()
        ));
        for role in ["export", "import"] {
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "imported_vectorized_bodies_keep_identity_when_merged_with_fresh_bodies",
                ])
                .env(ROLE, role)
                .env(PAYLOAD, &path)
                .status()
                .unwrap();
            assert!(status.success(), "fresh-process {role} failed");
        }
        fs::remove_file(path).unwrap();
        return;
    };
    let path = std::path::PathBuf::from(std::env::var_os(PAYLOAD).unwrap());
    if role == "export" {
        let bytes = bincode::encode_to_vec(evaluator(2), bincode::config::standard()).unwrap();
        fs::write(path, bytes).unwrap();
    } else {
        let bytes = fs::read(path).unwrap();
        let (mut imported, consumed): (ExpressionEvaluator<Number>, _) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(consumed, bytes.len());
        let fresh = evaluator(3);
        let old_functions = imported.export_instructions().sub_evaluators;
        let new_functions = fresh.export_instructions().sub_evaluators;
        assert!(old_functions.iter().all(|old| {
            new_functions
                .iter()
                .all(|new| old.symbol != new.symbol || old.tags != new.tags)
        }));
        imported.merge(fresh, Some(2)).unwrap();
        let mut numeric = imported.map_coeff(&|coefficient| coefficient.re.to_f64());
        let mut outputs = [0.; 4];
        numeric.evaluate(&[2., 1.], &mut outputs);
        assert_eq!(outputs, [4., 4., 8., 12.]);
    }
}
