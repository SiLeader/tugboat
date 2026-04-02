// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::data::StatusResponse;
use actix_web::body::BoxBody;
use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, TypeMeta};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ResourceList {
    #[serde(flatten)]
    type_meta: TypeMeta,
    #[serde(rename = "metadata")]
    object_meta: ObjectMeta,
    items: Vec<serde_json::Value>,
}

impl ResourceList {
    pub(crate) fn from_serializable<T: Serialize>(
        items: Vec<T>,
    ) -> Result<Self, Box<StatusResponse>> {
        match items
            .into_iter()
            .map(|v| serde_json::to_value(v))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(items) => Ok(Self::from_raw_items(items)),
            Err(_e) => Err(Box::new(StatusResponse::internal_error(
                "Failed to serialize data",
                None,
            ))),
        }
    }

    pub(crate) fn from_raw_items(items: Vec<serde_json::Value>) -> Self {
        Self {
            type_meta: TypeMeta {
                api_version: Some("v1".to_string()),
                kind: Some("List".to_string()),
            },
            object_meta: ObjectMeta::default(),
            items,
        }
    }
}

impl Responder for ResourceList {
    type Body = BoxBody;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        if wants_table_response(req) {
            HttpResponse::Ok().json(Table::from_raw_items(self.items))
        } else {
            self.into()
        }
    }
}

impl From<ResourceList> for HttpResponse {
    fn from(value: ResourceList) -> Self {
        HttpResponse::Ok().json(value)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Table {
    #[serde(flatten)]
    type_meta: TypeMeta,
    #[serde(rename = "metadata")]
    object_meta: ObjectMeta,
    column_definitions: Vec<TableColumnDefinition>,
    rows: Vec<TableRow>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TableColumnDefinition {
    name: String,
    #[serde(rename = "type")]
    type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    description: String,
    priority: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TableRow {
    cells: Vec<serde_json::Value>,
    object: serde_json::Value,
}

impl Table {
    pub(crate) fn from_raw_items(items: Vec<serde_json::Value>) -> Self {
        let include_namespace = items.iter().any(item_has_namespace);
        let kind = items
            .first()
            .and_then(|i| i.pointer("/kind").and_then(|k| k.as_str()))
            .map(String::from);

        let rows = items
            .into_iter()
            .map(|item| TableRow {
                cells: table_cells(&item, include_namespace, kind.as_deref()),
                object: item,
            })
            .collect();

        Self {
            type_meta: TypeMeta {
                api_version: Some("meta.k8s.io/v1".to_string()),
                kind: Some("Table".to_string()),
            },
            object_meta: ObjectMeta::default(),
            column_definitions: table_columns(include_namespace, kind.as_deref()),
            rows,
        }
    }

    pub(crate) fn from_serializable<T>(item: &T) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        Ok(Self::from_raw_items(vec![serde_json::to_value(item)?]))
    }
}

pub(crate) fn wants_table_response(req: &HttpRequest) -> bool {
    req.headers()
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("as=Table"))
}

fn table_columns(include_namespace: bool, kind: Option<&str>) -> Vec<TableColumnDefinition> {
    let mut columns = vec![TableColumnDefinition {
        name: "Name".to_string(),
        type_: "string".to_string(),
        format: Some("name".to_string()),
        description: "Resource name".to_string(),
        priority: 0,
    }];
    if include_namespace {
        columns.push(TableColumnDefinition {
            name: "Namespace".to_string(),
            type_: "string".to_string(),
            format: None,
            description: "Resource namespace".to_string(),
            priority: 0,
        });
    }

    match kind {
        Some("Ship") => {
            columns.push(TableColumnDefinition {
                name: "Class".to_string(),
                type_: "string".to_string(),
                format: None,
                description: "Ship Class".to_string(),
                priority: 0,
            });
            columns.push(TableColumnDefinition {
                name: "Node".to_string(),
                type_: "string".to_string(),
                format: None,
                description: "Node".to_string(),
                priority: 0,
            });
        }
        Some("Deployment") | Some("ReplicaSet") => {
            columns.push(TableColumnDefinition {
                name: "Replicas".to_string(),
                type_: "integer".to_string(),
                format: None,
                description: "Desired replicas".to_string(),
                priority: 0,
            });
            columns.push(TableColumnDefinition {
                name: "Available".to_string(),
                type_: "integer".to_string(),
                format: None,
                description: "Available replicas".to_string(),
                priority: 0,
            });
        }
        _ => {}
    }

    columns.push(TableColumnDefinition {
        name: "Created".to_string(),
        type_: "string".to_string(),
        format: None,
        description: "Creation timestamp".to_string(),
        priority: 0,
    });
    columns
}

fn item_has_namespace(item: &serde_json::Value) -> bool {
    item.pointer("/metadata/namespace")
        .is_some_and(|value| !value.is_null())
}

fn table_cells(
    item: &serde_json::Value,
    include_namespace: bool,
    kind: Option<&str>,
) -> Vec<serde_json::Value> {
    let mut cells = vec![
        item.pointer("/metadata/name")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    ];
    if include_namespace {
        cells.push(
            item.pointer("/metadata/namespace")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
    }

    match kind {
        Some("Ship") => {
            cells.push(
                item.pointer("/spec/shipClass")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            );
            cells.push(
                item.pointer("/spec/nodeName")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            );
        }
        Some("Deployment") | Some("ReplicaSet") => {
            cells.push(
                item.pointer("/spec/replicas")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!(1)),
            );
            cells.push(
                item.pointer("/status/readyReplicas")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!(0)),
            );
        }
        _ => {}
    }

    cells.push(
        item.pointer("/metadata/creationTimestamp")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    );
    cells
}

#[cfg(test)]
mod tests {
    use super::{ResourceList, Table, wants_table_response};
    use actix_web::Responder;
    use actix_web::body::to_bytes;
    use actix_web::http::header;
    use actix_web::test::TestRequest;
    use serde_json::Value;

    #[actix_web::test]
    async fn resource_list_returns_table_when_requested() {
        let req = TestRequest::default()
            .insert_header((
                header::ACCEPT,
                "application/json;as=Table;v=v1;g=meta.k8s.io",
            ))
            .to_http_request();

        let response = ResourceList::from_raw_items(vec![serde_json::json!({
            "apiVersion": "v1",
            "kind": "Ship",
            "metadata": {
                "name": "demo",
                "namespace": "default",
                "creationTimestamp": "2026-04-02T05:00:00Z"
            }
        })])
        .respond_to(&req);

        let body = to_bytes(response.into_body())
            .await
            .expect("body should be readable");
        let json: Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json["kind"], "Table");
        assert_eq!(json["apiVersion"], "meta.k8s.io/v1");
        assert_eq!(json["columnDefinitions"][0]["name"], "Name");
        assert_eq!(json["columnDefinitions"][1]["name"], "Namespace");
        assert_eq!(json["rows"][0]["cells"][0], "demo");
        assert_eq!(json["rows"][0]["cells"][1], "default");
    }

    #[test]
    fn serializable_table_uses_single_row() {
        let table = Table::from_serializable(&serde_json::json!({
            "metadata": {
                "name": "single",
                "creationTimestamp": "2026-04-02T05:00:00Z"
            }
        }))
        .expect("table serialization should succeed");

        let json = serde_json::to_value(table).expect("table should serialize");
        assert_eq!(json["rows"].as_array().map(Vec::len), Some(1));
        assert_eq!(json["rows"][0]["cells"][0], "single");
    }

    #[test]
    fn detects_table_accept_header() {
        let req = TestRequest::default()
            .insert_header((
                header::ACCEPT,
                "application/json;as=Table;v=v1;g=meta.k8s.io, application/json",
            ))
            .to_http_request();
        assert!(wants_table_response(&req));
    }
}
