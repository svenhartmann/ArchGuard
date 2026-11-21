//! Services → API contracts: the web viewer's `renderApiContracts`
//! (`viewer_engineering_quality.js`) over `archguard_engine::read_api_contracts`,
//! the engine behind `archguard contracts` and MCP `api_contracts`.

use std::collections::BTreeSet;

use archguard_engine::{ApiContract, ApiContractReport, ApiOperation};

use crate::tab_support::model;
use crate::ui::{SvcChange, SvcContract, SvcContractDetail, SvcContracts, SvcOperation};

/// Contracts, operations and changes per page, as the web's.
pub const PAGE: usize = 20;

/// The section's state; `Default` while the contracts are not read.
#[derive(Default)]
pub struct Contracts {
    report: Option<ApiContractReport>,
    /// The open contract (index into the report).
    open: Option<usize>,
    limit: usize,
    /// Operations and changes shown of the open contract.
    detail: usize,
}

fn operation(c: &ApiContract, op: &ApiOperation) -> SvcOperation {
    let schemas: BTreeSet<&str> = op
        .request_references
        .iter()
        .chain(&op.response_references)
        .map(String::as_str)
        .collect();
    let users = c
        .usages
        .iter()
        .filter(|u| u.operation == op.id || u.operation == "*");
    let (mapped, unmapped): (Vec<_>, Vec<_>) = users.partition(|u| u.mapped_service);
    let names = |list: Vec<&archguard_engine::ApiUsage>| {
        list.iter()
            .map(|u| u.consumer.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    SvcOperation {
        title: match &op.operation_id {
            Some(id) => format!("{} · {id}", op.id),
            None => op.id.clone(),
        }
        .into(),
        pointer: if op.pointer.is_empty() {
            String::new()
        } else {
            format!("{}#{}", c.source_path, op.pointer)
        }
        .into(),
        schemas: schemas.into_iter().collect::<Vec<_>>().join(", ").into(),
        consumers: names(mapped).into(),
        unmapped: names(unmapped).into(),
    }
}

impl Contracts {
    pub fn new(report: ApiContractReport) -> Self {
        Contracts {
            report: Some(report),
            open: None,
            limit: PAGE,
            detail: PAGE,
        }
    }

    /// Open listed contract `i`, or close it when it is open.
    pub fn choose(&mut self, i: usize) {
        let len = self.report.as_ref().map_or(0, |r| r.contracts.len());
        self.open = (self.open != Some(i) && i < len).then_some(i);
        self.detail = PAGE;
    }

    pub fn more(&mut self) {
        self.limit += PAGE;
    }

    pub fn more_details(&mut self) {
        self.detail += PAGE;
    }

    pub fn model(&self) -> SvcContracts {
        let Some(report) = &self.report else {
            return SvcContracts::default();
        };
        let rows = report
            .contracts
            .iter()
            .take(self.limit)
            .enumerate()
            .map(|(i, c)| SvcContract {
                provider: c.provider.as_str().into(),
                source: format!(
                    "{} · {} · {}",
                    c.source_path, c.source_status, c.comparison_status
                )
                .into(),
                operations: c.operations.len() as i32,
                callers: c.configured_callers.join(", ").into(),
                usage_status: c.usage_status.as_str().into(),
                mapped: c.mapped_service,
                no_baseline: c.comparison_status == "no_baseline",
                open: self.open == Some(i),
            });
        SvcContracts {
            state: 1,
            status: report.status.as_str().into(),
            total: report.contracts.len() as i32,
            contracts: model(rows.collect::<Vec<_>>()),
            more: report.contracts.len() > self.limit,
            detail: self
                .open
                .and_then(|i| report.contracts.get(i))
                .map(|c| self.detail(c))
                .unwrap_or_default(),
            notes: model(
                report
                    .limitations
                    .iter()
                    .chain(&report.diagnostics)
                    .take(20)
                    .map(|n| n.as_str().into()),
            ),
        }
    }

    fn detail(&self, c: &ApiContract) -> SvcContractDetail {
        let changes = c.changes.iter().take(self.detail).map(|ch| SvcChange {
            title: format!("{} · {} · {}", ch.operation, ch.severity, ch.kind).into(),
            message: ch.message.as_str().into(),
            consumers: ch.consumers.join(", ").into(),
        });
        SvcContractDetail {
            operations: model(
                c.operations
                    .iter()
                    .take(self.detail)
                    .map(|op| operation(c, op)),
            ),
            changes: model(changes),
            more: c.operations.len().max(c.changes.len()) > self.detail,
            notes: model(c.diagnostics.iter().map(|d| d.as_str().into())),
        }
    }
}
