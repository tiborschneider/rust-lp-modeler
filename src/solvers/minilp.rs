use dsl::{LpObjective, LpProblem, LpConstraint, LpExpression, Constraint, LpExprNode, LpContinuous};
use std::collections::HashMap;
use solvers::{SolverTrait, Solution, Status};
use dsl::LpExprNode::LitVal;
use dsl::LpExprOp::{Multiplication, Addition, Subtraction};

fn direction_to_minilp(objective: &LpObjective) -> minilp::OptimizationDirection {
    match objective {
        LpObjective::Maximize => minilp::OptimizationDirection::Maximize,
        LpObjective::Minimize => minilp::OptimizationDirection::Minimize,
    }
}

fn add_constraint_to_minilp(
    constraint: &LpConstraint,
    variables: &mut HashMap<String, minilp::Variable>,
    pb: &mut minilp::Problem,
) -> Result<(), String> {
    let LpConstraint(expr, op, constant_arena) = constraint.clone();
    let constant = if let &LitVal(c) = constant_arena.get_root_expr_ref() { c } else {
        return Err("not properly simplified".into());
    };
    let expr_variables = decompose_expression(expr)?;
    let mut expr = minilp::LinearExpr::empty();
    for (name, coefficient) in expr_variables.0 {
        let var = variables.entry(name).or_insert_with(|| {
            pb.add_var(0., (f64::NEG_INFINITY, f64::INFINITY))
        }).clone();
        expr.add(var, coefficient.coefficient.into());
    }
    let op = comparison_to_minilp(op);
    pb.add_constraint(expr, op, f64::from(constant));
    Ok(())
}

fn comparison_to_minilp(op: Constraint) -> minilp::ComparisonOp {
    match op {
        Constraint::GreaterOrEqual => minilp::ComparisonOp::Ge,
        Constraint::LessOrEqual => minilp::ComparisonOp::Le,
        Constraint::Equal => minilp::ComparisonOp::Eq
    }
}

#[derive(Debug, PartialEq)]
struct VarWithCoeff {
    coefficient: f64,
    min: f64,
    max: f64,
}

impl Default for VarWithCoeff {
    fn default() -> Self {
        VarWithCoeff { coefficient: 0., min: f64::NEG_INFINITY, max: f64::INFINITY }
    }
}

#[derive(Debug, Default, PartialEq)]
struct VarList(HashMap<String, VarWithCoeff>);

impl VarList {
    fn add(&mut self, var: LpContinuous, coefficient: f64) {
        let LpContinuous { name, lower_bound, upper_bound } = var;
        let prev = self.0.entry(name).or_default();
        prev.coefficient += coefficient;
        if let Some(lower) = lower_bound {
            prev.min = prev.min.max(lower);
        }
        if let Some(upper) = upper_bound {
            prev.max = prev.max.min(upper);
        }
    }
}

fn decompose_expression(
    mut expr: LpExpression,
) -> Result<VarList, String> {
    expr.simplify();
    let mut decomposed = VarList::default();
    let mut idxs = vec![(1., expr.get_root_index())];
    while let Some((factor, idx)) = idxs.pop() {
        match expr.expr_ref_at(idx) {
            LpExprNode::ConsCont(var) => { decomposed.add(var.clone(), factor) }
            &LpExprNode::LpCompExpr(Multiplication, lhs, rhs) => {
                if let &LpExprNode::LitVal(lit) = expr.expr_ref_at(lhs) {
                    idxs.push((factor * lit, rhs))
                } else {
                    return Err(format!("Non-simplified multiplication: {:?}", expr.expr_ref_at(idx)));
                }
            }
            &LpExprNode::LpCompExpr(Addition, lhs, rhs) => {
                idxs.push((factor, lhs));
                idxs.push((factor, rhs));
            }
            &LpExprNode::LpCompExpr(Subtraction, lhs, rhs) => {
                idxs.push((factor, lhs));
                idxs.push((-factor, rhs));
            }
            x => return Err(format!("Unsupported expression: {:?}", x))
        }
    }
    Ok(decomposed)
}


/// Returns a map from dsl variable name to minilp variable
fn add_objective_to_minilp(
    objective: LpExpression,
    pb: &mut minilp::Problem,
) -> Result<HashMap<String, minilp::Variable>, String> {
    let vars = decompose_expression(objective)?;
    Ok(vars.0.into_iter()
        .map(|(name, VarWithCoeff { coefficient, min, max })| {
            let var = pb.add_var(
                coefficient.into(),
                (min.into(), max.into()),
            );
            (name, var)
        }).collect()
    )
}

fn problem_to_minilp(pb: &LpProblem) -> Result<(minilp::Problem, Vec<Option<String>>), String> {
    let objective = direction_to_minilp(&pb.objective_type);
    let mut minilp_pb = minilp::Problem::new(objective);
    let objective = pb.obj_expr_arena.clone().ok_or("Missing objective")?;
    let mut minilp_variables = add_objective_to_minilp(objective, &mut minilp_pb)?;
    for constraint in &pb.constraints {
        add_constraint_to_minilp(
            constraint,
            &mut minilp_variables,
            &mut minilp_pb,
        )?;
    }
    let mut ordered_vars = vec![None; minilp_variables.len()];
    for (name, var) in minilp_variables {
        ordered_vars[var.idx()] = Some(name);
    }
    Ok((minilp_pb, ordered_vars))
}

pub struct MiniLpSolver;

impl MiniLpSolver {
    pub fn new() -> Self { Self }
}

impl SolverTrait for MiniLpSolver {
    type P = LpProblem;

    fn run<'a>(&self, problem: &'a Self::P) -> Result<Solution<'a>, String> {
        let (minilp_pb, variable_names) = problem_to_minilp(problem)?;
        let minilp_result = minilp_pb.solve();
        solution_from_minilp(minilp_result, variable_names)
    }
}

fn solution_from_minilp(
    result: Result<minilp::Solution, minilp::Error>,
    mut variable_names: Vec<Option<String>>,
) -> Result<Solution<'static>, String> {
    match result {
        Ok(solution) => {
            let results: Option<HashMap<String, f64>> = solution.iter()
                .map(|(var, &value)| {
                    std::mem::take(&mut variable_names[var.idx()]).map(|name| {
                        (name, value as f64)
                    })
                })
                .collect();
            if let Some(results) = results {
                Ok(Solution::new(Status::Optimal, results))
            } else {
                Err("missing variable name".into())
            }
        }
        Err(minilp::Error::Unbounded) => {
            Ok(Solution::new(Status::Unbounded, HashMap::new()))
        }
        Err(minilp::Error::Infeasible) => {
            Ok(Solution::new(Status::Infeasible, HashMap::new()))
        }
    }
}

#[test]
fn test_decompose() {
    let ref a = LpContinuous::new("a");
    let ref b = LpContinuous::new("b");
    let expr = (4 * (3 * a - b * 2 + a)) * 1 + b;
    let decomposed = decompose_expression(expr);
    let mut expected = VarList::default();
    expected.add(a.clone(), 4. * 3. + 4.);
    expected.add(b.clone(), 4. * (-2.) + 1.);
    assert_eq!(decomposed, Ok(expected));
}

#[test]
fn test_solve() {
    use dsl::operations::LpOperations;
    let ref a = LpContinuous::new("a");
    let ref b = LpContinuous::new("b");

    // Define problem and objective sense
    let mut problem = LpProblem::new("One Problem", LpObjective::Maximize);
    problem += 10 * a + 20 * b;
    problem += (500 * a - 1000 * b).ge(10000);
    problem += (a).le(b);

    let expected: HashMap<String, f64> = vec![
        ("a".into(), -20.),
        ("b".into(), -20.)
    ].into_iter().collect();
    let actual = MiniLpSolver::new().run(&problem).expect("could not solve").results;
    assert_eq!(actual, expected);
}

#[test]
fn decompose_large() {
    use dsl::lp_sum;
    let count = 1000;
    let vars: Vec<LpExpression> = (0..count)
        .map(|i|
            &LpContinuous::new(&format!("v{}", i)) * 2
        )
        .collect();
    let sum = lp_sum(&vars);
    let vars = decompose_expression(sum).expect("decompose failed");
    assert_eq!(vars.0.keys().len(), count);
}