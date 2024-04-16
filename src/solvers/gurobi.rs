extern crate uuid;
use self::uuid::Uuid;

use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::process::Command;

use dsl::LpProblem;
use format::lp_format::*;
use solvers::{Solution, SolverTrait, SolverWithSolutionParsing, Status};

pub struct GurobiSolver {
    name: String,
    command_name: String,
    temp_solution_file: String,
}

impl GurobiSolver {
    pub fn new() -> GurobiSolver {
        GurobiSolver {
            name: "Gurobi".to_string(),
            command_name: "gurobi_cl".to_string(),
            temp_solution_file: format!("{}.sol", Uuid::new_v4().to_string()),
        }
    }
    pub fn command_name(&self, command_name: String) -> GurobiSolver {
        GurobiSolver {
            name: self.name.clone(),
            command_name,
            temp_solution_file: self.temp_solution_file.clone(),
        }
    }
}

impl SolverWithSolutionParsing for GurobiSolver {
    fn read_specific_solution<'a>(
        &self,
        f: &File,
        problem: Option<&'a LpProblem>,
    ) -> Result<Solution<'a>, String> {
        let mut vars_value: HashMap<_, _> = HashMap::new();
        let mut file = BufReader::new(f);
        let mut buffer = String::new();
        let _ = file.read_line(&mut buffer);

        if let Some(_) = buffer.split(" ").next() {
            for line in file.lines() {
                let l = line.unwrap();

                // Gurobi version 7 add comments on the header file
                if let Some('#') = l.chars().next() {
                    continue;
                }

                let result_line: Vec<_> = l.split_whitespace().collect();
                if result_line.len() == 2 {
                    match result_line[1].parse::<f32>() {
                        Ok(n) => {
                            vars_value.insert(result_line[0].to_string(), n);
                        }
                        Err(e) => return Err(format!("{}", e.to_string())),
                    }
                } else {
                    return Err("Incorrect solution format".to_string());
                }
            }
        } else {
            return Err("Incorrect solution format".to_string());
        }
        // TODO/FIX: always optimal if no err...
        if let Some(p) = problem {
            Ok(Solution::with_problem(Status::Optimal, vars_value, p))
        } else {
            Ok(Solution::new(Status::Optimal, vars_value))
        }
    }
}

impl SolverTrait for GurobiSolver {
    type P = LpProblem;
    fn run<'a>(&self, problem: &'a Self::P) -> Result<Solution<'a>, String> {
        let file_model = &format!("{}.lp", problem.unique_name);

        match problem.write_lp(file_model) {
            Ok(_) => {
                let result = match Command::new(&self.command_name)
                    .arg(format!("ResultFile={}", self.temp_solution_file))
                    .arg(file_model)
                    .output()
                {
                    Ok(r) => {
                        if r.status.success() {
                            let mut status = Status::SubOptimal;
                            let result = String::from_utf8(r.stdout).expect("");
                            if result.contains("Optimal solution found") {
                                status = Status::Optimal;
                            } else if result.contains("infesible") {
                                status = Status::Infeasible;
                            }
                            self.read_solution(&self.temp_solution_file, Some(problem)).map(
                                |solution| Solution {
                                    status,
                                    ..solution.clone()
                                },
                            )
                        } else {
                            Err(format!(
                                "{} exited with {}\n\nSTDOUT:\n{}\n\nSTDERR:\n{}\n\n",
                                self.command_name,
                                r.status,
                                String::from_utf8_lossy(&r.stdout),
                                String::from_utf8_lossy(&r.stderr),
                            ))
                        }
                    }
                    Err(_) => Err(format!("Error running the {} solver", self.name)),
                };
                let _ = fs::remove_file(&file_model);

                result
            }
            Err(e) => Err(e.to_string()),
        }
    }
}
