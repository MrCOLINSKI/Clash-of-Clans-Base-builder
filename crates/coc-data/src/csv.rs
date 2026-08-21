//! Parser for Supercell's CSV logic tables.
//!
//! These files are not ordinary CSVs. The layout is:
//!
//! ```text
//! row 0        column names
//! row 1        column types ("String" | "int" | "boolean")
//! row 2..      data rows, grouped into per-entity blocks
//! ```
//!
//! An entity block starts at any row whose first cell is non-empty. Within a
//! block each subsequent row is one level of that entity, and a blank cell
//! **inherits the last non-blank value seen for that column within the same
//! block** (carry-forward). Carry state resets at every block boundary, so a
//! value never leaks from one entity into the next.
//!
//! Getting this wrong yields null stats for every level above 1, which is why
//! [`tests`] covers it directly and `tests/carryover.rs` covers it against the
//! real shipped tables.

use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CsvError {
    #[error("table has {0} rows; need at least a header and a type row")]
    TooFewRows(usize),
    #[error("column `{0}` not present in table")]
    NoSuchColumn(String),
    #[error("column `{column}` value `{value}` is not an integer")]
    NotAnInt { column: String, value: String },
    #[error("column `{column}` value `{value}` is not a boolean")]
    NotABool { column: String, value: String },
}

/// Declared type of a column, taken from the table's type row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    String,
    Int,
    Bool,
    /// Type row carried something we do not recognise; treated as a string.
    Unknown,
}

impl ColumnType {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "string" => ColumnType::String,
            "int" => ColumnType::Int,
            "boolean" | "bool" => ColumnType::Bool,
            _ => ColumnType::Unknown,
        }
    }
}

/// One fully carry-over-resolved level of an entity.
#[derive(Debug, Clone)]
pub struct Row {
    cells: Vec<Option<String>>,
    columns: std::sync::Arc<HashMap<String, usize>>,
}

impl Row {
    /// Raw resolved value for a column, or `None` if blank everywhere above it.
    pub fn get(&self, column: &str) -> Option<&str> {
        let idx = *self.columns.get(column)?;
        self.cells.get(idx)?.as_deref()
    }

    /// Resolved value, erroring if the column does not exist at all.
    ///
    /// Distinguishes "no such column" (a schema drift bug) from "column is
    /// blank for this entity" (legitimate, e.g. `AttackRange` on a Wall).
    pub fn try_get(&self, column: &str) -> Result<Option<&str>, CsvError> {
        let idx = *self
            .columns
            .get(column)
            .ok_or_else(|| CsvError::NoSuchColumn(column.to_string()))?;
        Ok(self.cells.get(idx).and_then(|c| c.as_deref()))
    }

    pub fn get_i64(&self, column: &str) -> Result<Option<i64>, CsvError> {
        match self.try_get(column)? {
            None => Ok(None),
            Some(v) => v
                .parse::<i64>()
                .map(Some)
                .map_err(|_| CsvError::NotAnInt {
                    column: column.to_string(),
                    value: v.to_string(),
                }),
        }
    }

    pub fn get_bool(&self, column: &str) -> Result<Option<bool>, CsvError> {
        match self.try_get(column)? {
            None => Ok(None),
            Some(v) => match v.trim().to_ascii_lowercase().as_str() {
                "true" | "1" => Ok(Some(true)),
                "false" | "0" => Ok(Some(false)),
                _ => Err(CsvError::NotABool {
                    column: column.to_string(),
                    value: v.to_string(),
                }),
            },
        }
    }

    /// Integer value, or `default` when the column is blank for this entity.
    pub fn i64_or(&self, column: &str, default: i64) -> Result<i64, CsvError> {
        Ok(self.get_i64(column)?.unwrap_or(default))
    }

    /// Integer value for a column that may not exist in this table at all.
    ///
    /// Use only where two tables share a parser but not their full schema
    /// (`characters.csv` and `heroes.csv`). Everywhere else a missing column
    /// is schema drift and must stay a hard error, so this is deliberately
    /// separate from [`Row::get_i64`] rather than a lenient default.
    pub fn i64_if_present(&self, column: &str) -> Result<Option<i64>, CsvError> {
        if self.columns.contains_key(column) {
            self.get_i64(column)
        } else {
            Ok(None)
        }
    }

    /// Boolean value for a column that may not exist in this table at all.
    ///
    /// See [`Row::i64_if_present`] for when this is appropriate.
    pub fn bool_if_present(&self, column: &str, default: bool) -> Result<bool, CsvError> {
        if self.columns.contains_key(column) {
            self.bool_or(column, default)
        } else {
            Ok(default)
        }
    }

    /// String value for a column that may not exist in this table at all.
    pub fn str_if_present(&self, column: &str) -> Option<&str> {
        if self.columns.contains_key(column) {
            self.get(column)
        } else {
            None
        }
    }

    /// Boolean value, or `default` when the column is blank for this entity.
    pub fn bool_or(&self, column: &str, default: bool) -> Result<bool, CsvError> {
        Ok(self.get_bool(column)?.unwrap_or(default))
    }
}

/// One entity (a building, troop, spell, ...) and all of its levels.
#[derive(Debug, Clone)]
pub struct Entity {
    pub name: String,
    /// One entry per level, in file order. Index 0 is level 1.
    pub levels: Vec<Row>,
}

impl Entity {
    /// Level count for this entity.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Row for a 1-based level number.
    pub fn level(&self, level: usize) -> Option<&Row> {
        level.checked_sub(1).and_then(|i| self.levels.get(i))
    }
}

/// A parsed logic table.
#[derive(Debug, Clone)]
pub struct Table {
    pub columns: Vec<String>,
    pub types: Vec<ColumnType>,
    pub entities: Vec<Entity>,
    index: std::sync::Arc<HashMap<String, usize>>,
    by_name: HashMap<String, usize>,
}

impl Table {
    /// Looks up an entity by its `Name` column.
    pub fn entity(&self, name: &str) -> Option<&Entity> {
        self.by_name.get(name).and_then(|&i| self.entities.get(i))
    }

    pub fn has_column(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn column_type(&self, name: &str) -> Option<ColumnType> {
        self.index.get(name).and_then(|&i| self.types.get(i)).copied()
    }

    /// Total number of level rows across every entity.
    pub fn row_count(&self) -> usize {
        self.entities.iter().map(|e| e.levels.len()).sum()
    }

    /// Parses a decoded CSV table, applying carry-over resolution.
    pub fn parse(text: &str) -> Result<Table, CsvError> {
        let raw_rows = tokenize(text);
        if raw_rows.len() < 2 {
            return Err(CsvError::TooFewRows(raw_rows.len()));
        }

        let columns: Vec<String> =
            raw_rows[0].iter().map(|c| c.trim().to_string()).collect();
        let types: Vec<ColumnType> = {
            let mut t: Vec<ColumnType> =
                raw_rows[1].iter().map(|c| ColumnType::parse(c)).collect();
            t.resize(columns.len(), ColumnType::Unknown);
            t
        };

        let index: HashMap<String, usize> = columns
            .iter()
            .enumerate()
            .map(|(i, c)| (c.clone(), i))
            .collect();
        let index = std::sync::Arc::new(index);

        let width = columns.len();
        let mut entities: Vec<Entity> = Vec::new();
        // Carry state for the block currently being read. Reset on each new
        // entity so values never leak across the block boundary.
        let mut carry: Vec<Option<String>> = vec![None; width];

        for raw in raw_rows.iter().skip(2) {
            // An empty *line* is a separator or a trailing newline. A full-width
            // row of empty *cells* is a real level whose every stat is inherited
            // from the level above, so the two must not be conflated: dropping
            // the latter would silently lose a level and shift every level
            // number after it.
            let is_empty_line =
                raw.len() <= 1 && raw.first().is_none_or(|c| c.trim().is_empty());
            if is_empty_line {
                continue;
            }

            let first = raw.first().map(|c| c.trim()).unwrap_or("");
            if !first.is_empty() {
                entities.push(Entity {
                    name: first.to_string(),
                    levels: Vec::new(),
                });
                carry = vec![None; width];
            }

            // A data row before any named entity has nothing to attach to.
            let Some(current) = entities.last_mut() else {
                continue;
            };

            let mut cells: Vec<Option<String>> = Vec::with_capacity(width);
            for col in 0..width {
                let raw_cell = raw.get(col).map(|c| c.trim()).unwrap_or("");
                if raw_cell.is_empty() {
                    // Blank: inherit whatever this column last held in-block.
                    cells.push(carry[col].clone());
                } else {
                    carry[col] = Some(raw_cell.to_string());
                    cells.push(Some(raw_cell.to_string()));
                }
            }

            current.levels.push(Row {
                cells,
                columns: index.clone(),
            });
        }

        let by_name = entities
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.clone(), i))
            .collect();

        Ok(Table {
            columns,
            types,
            entities,
            index,
            by_name,
        })
    }
}

/// Splits CSV text into rows of cells, honouring quoted fields.
///
/// Supercell tables use `"` quoting with `""` as an embedded quote, and may
/// use either LF or CRLF line endings.
fn tokenize(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => row.push(std::mem::take(&mut cell)),
            '\r' if !in_quotes => {}
            '\n' if !in_quotes => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the real shape of `buildings.csv`: entity-level columns appear
    /// only on the block's first row, per-level columns on every row.
    const SAMPLE: &str = "\
\"Name\",\"BuildingClass\",\"Width\",\"Hitpoints\",\"AttackRange\"
\"String\",\"String\",\"int\",\"int\",\"int\"
\"Cannon\",\"Defense\",\"3\",\"300\",\"900\"
\"\",\"\",\"\",\"360\",\"\"
\"\",\"\",\"\",\"420\",\"\"
\"Mortar\",\"Defense\",\"3\",\"400\",\"1100\"
\"\",\"\",\"\",\"450\",\"\"
";

    fn table() -> Table {
        Table::parse(SAMPLE).expect("sample parses")
    }

    #[test]
    fn splits_entities_into_blocks() {
        let t = table();
        assert_eq!(t.entities.len(), 2);
        assert_eq!(t.entity("Cannon").unwrap().level_count(), 3);
        assert_eq!(t.entity("Mortar").unwrap().level_count(), 2);
    }

    #[test]
    fn carries_entity_columns_to_higher_levels() {
        let t = table();
        let cannon = t.entity("Cannon").unwrap();
        // The bug this guards: level 2+ reading back as null.
        for lvl in 1..=3 {
            let row = cannon.level(lvl).unwrap();
            assert_eq!(row.get("BuildingClass"), Some("Defense"), "level {lvl}");
            assert_eq!(row.get_i64("Width").unwrap(), Some(3), "level {lvl}");
            assert_eq!(
                row.get_i64("AttackRange").unwrap(),
                Some(900),
                "level {lvl}"
            );
        }
    }

    #[test]
    fn per_level_columns_are_not_overwritten_by_carry() {
        let t = table();
        let cannon = t.entity("Cannon").unwrap();
        let hp: Vec<_> = (1..=3)
            .map(|l| cannon.level(l).unwrap().get_i64("Hitpoints").unwrap())
            .collect();
        assert_eq!(hp, vec![Some(300), Some(360), Some(420)]);
    }

    #[test]
    fn carry_state_resets_between_entities() {
        let t = table();
        let mortar = t.entity("Mortar").unwrap();
        // Must be Mortar's 1100, never Cannon's carried 900.
        assert_eq!(
            mortar.level(1).unwrap().get_i64("AttackRange").unwrap(),
            Some(1100)
        );
        assert_eq!(
            mortar.level(2).unwrap().get_i64("AttackRange").unwrap(),
            Some(1100)
        );
        assert_eq!(
            mortar.level(2).unwrap().get_i64("Hitpoints").unwrap(),
            Some(450)
        );
    }

    #[test]
    fn carry_forward_uses_most_recent_value_not_block_first() {
        // A column set on level 1 and again on level 3 must hold the level-3
        // value at level 4, not fall back to the level-1 value.
        let csv = "\
\"Name\",\"Mode\"
\"String\",\"String\"
\"X\",\"a\"
\"\",\"\"
\"\",\"b\"
\"\",\"\"
";
        let t = Table::parse(csv).unwrap();
        let x = t.entity("X").unwrap();
        let modes: Vec<_> = (1..=4).map(|l| x.level(l).unwrap().get("Mode")).collect();
        assert_eq!(modes, vec![Some("a"), Some("a"), Some("b"), Some("b")]);
    }

    #[test]
    fn blank_column_stays_none_rather_than_defaulting() {
        let csv = "\
\"Name\",\"AttackRange\"
\"String\",\"int\"
\"Wall\",\"\"
";
        let t = Table::parse(csv).unwrap();
        let row = t.entity("Wall").unwrap().level(1).unwrap();
        assert_eq!(row.get_i64("AttackRange").unwrap(), None);
        assert_eq!(row.i64_or("AttackRange", -1).unwrap(), -1);
    }

    #[test]
    fn missing_column_is_distinguishable_from_blank_cell() {
        let t = table();
        let row = t.entity("Cannon").unwrap().level(1).unwrap();
        assert!(matches!(
            row.try_get("NoSuchThing"),
            Err(CsvError::NoSuchColumn(_))
        ));
    }

    #[test]
    fn handles_quoted_commas_and_escaped_quotes() {
        let csv = "\
\"Name\",\"TID\"
\"String\",\"String\"
\"A, B\",\"say \"\"hi\"\"\"
";
        let t = Table::parse(csv).unwrap();
        let e = &t.entities[0];
        assert_eq!(e.name, "A, B");
        assert_eq!(e.level(1).unwrap().get("TID"), Some("say \"hi\""));
    }

    #[test]
    fn parses_booleans_case_insensitively() {
        let csv = "\
\"Name\",\"AirTargets\"
\"String\",\"boolean\"
\"A\",\"TRUE\"
\"B\",\"false\"
";
        let t = Table::parse(csv).unwrap();
        assert_eq!(
            t.entity("A").unwrap().level(1).unwrap().get_bool("AirTargets").unwrap(),
            Some(true)
        );
        assert_eq!(
            t.entity("B").unwrap().level(1).unwrap().get_bool("AirTargets").unwrap(),
            Some(false)
        );
    }

    #[test]
    fn records_column_types_from_type_row() {
        let t = table();
        assert_eq!(t.column_type("Width"), Some(ColumnType::Int));
        assert_eq!(t.column_type("BuildingClass"), Some(ColumnType::String));
    }

    #[test]
    fn crlf_line_endings_parse_identically() {
        let t = Table::parse(&SAMPLE.replace('\n', "\r\n")).unwrap();
        assert_eq!(t.entities.len(), 2);
        assert_eq!(t.entity("Cannon").unwrap().level_count(), 3);
    }
}
