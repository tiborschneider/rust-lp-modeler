extern crate uuid;
use self::uuid::Uuid;

use std::fs;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Error, BufReader, BufRead};
use std::process::Command;

use dsl::LpProblem;
use format::lp_format::*;
use solvers::{Status, SolverTrait, SolverWithSolutionParsing, Solution};

pub struct GlpkSolver {
    name: String,
    command_name: String,
    temp_solution_file: String,
}

impl GlpkSolver {
    pub fn new() -> GlpkSolver {
        GlpkSolver {
            name: "Glpk".to_string(),
            command_name: "glpsol".to_string(),
            temp_solution_file: format!("{}.sol", Uuid::new_v4().to_string()),
        }
    }
    pub fn command_name(&self, command_name: String) -> GlpkSolver {
        GlpkSolver {
            name: self.name.clone(),
            command_name,
            temp_solution_file: self.temp_solution_file.clone(),
        }
    }
    pub fn with_temp_solution_file(&self, temp_solution_file: String) -> GlpkSolver {
        GlpkSolver {
            name: self.name.clone(),
            command_name: self.command_name.clone(),
            temp_solution_file,
        }
    }
}

impl SolverWithSolutionParsing for GlpkSolver {
    fn read_specific_solution<'a>(&self, f: &File, problem: Option<&'a LpProblem>) -> Result<Solution<'a>, String> {
        fn read_size(line: Option<Result<String, Error>>) -> Result<usize, String> {
            match line {
                Some(Ok(l)) => match l.split_whitespace().nth(1) {
                    Some(value) => match value.parse::<usize>() {
                        Ok(v) => Ok(v),
                        _ => return Err("Incorrect solution format".to_string()),
                    },
                    _ => return Err("Incorrect solution format".to_string()),
                },
                _ => return Err("Incorrect solution format".to_string()),
            }
        }
        let mut vars_value: HashMap<_, _> = HashMap::new();

        let file = BufReader::new(f);

        let mut iter = file.lines();
        let row = match read_size(iter.nth(1)) {
            Ok(value) => value,
            Err(e) => return Err(e.to_string()),
        };
        let col = match read_size(iter.nth(0)) {
            Ok(value) => value,
            Err(e) => return Err(e.to_string()),
        };
        let status = match iter.nth(1) {
            Some(Ok(status_line)) => match &status_line[12..] {
                "INTEGER OPTIMAL" | "OPTIMAL" => Status::Optimal,
                "INFEASIBLE (FINAL)" | "INTEGER EMPTY" => Status::Infeasible,
                "UNDEFINED" => Status::NotSolved,
                "INTEGER UNDEFINED" | "UNBOUNDED" => Status::Unbounded,
                _ => {
                    return Err("Incorrect solution format: Unknown solution status".to_string())
                }
            },
            _ => return Err("Incorrect solution format: No solution status found".to_string()),
        };
        let mut result_lines = iter.skip(row + 7);
        for _ in 0..col {
            let line = match result_lines.next() {
                Some(Ok(l)) => l,
                _ => {
                    return Err(
                        "Incorrect solution format: Not all columns are present".to_string()
                    )
                }
            };
            let result_line: Vec<_> = line.split_whitespace().collect();
            if result_line.len() >= 4 {
                match result_line[3].parse::<f64>() {
                    Ok(n) => {
                        vars_value.insert(result_line[1].to_string(), n);
                    }
                    Err(e) => return Err(e.to_string()),
                }
            } else {
                return Err(
                    "Incorrect solution format: Column specification has to few fields"
                        .to_string(),
                );
            }
        }
        if let Some(p) = problem {
            Ok( Solution::with_problem(status, vars_value, p) )
        } else {
            Ok( Solution::new(status, vars_value) )
        }
    }
}

impl SolverTrait for GlpkSolver {
    type P = LpProblem;
    fn run<'a>(&self, problem: &'a Self::P) -> Result<Solution<'a>, String> {
        let file_model = &format!("{}.lp", problem.unique_name);

        match problem.write_lp(file_model) {
            Ok(_) => {
                let result = match Command::new(&self.command_name)
                    .arg("--lp")
                    .arg(file_model)
                    .arg("-o")
                    .arg(&self.temp_solution_file)
                    .output()
                    {
                        Ok(r) => {
                            if r.status.success() {
                                self.read_solution(&self.temp_solution_file, Some(problem))
                            } else {
                                Err(r.status.to_string())
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
