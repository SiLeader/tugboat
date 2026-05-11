# Role-Based Access Control (RBAC)

Tugboat provides a comprehensive Role-Based Access Control (RBAC) system, heavily inspired by Kubernetes. It allows cluster administrators to define fine-grained permissions for users, groups, and service accounts.

## Core Concepts

RBAC in Tugboat is built around four primary resources:

- **Role**: Defines a set of permissions within a specific namespace.
- **ClusterRole**: Defines permissions across the entire cluster.
- **RoleBinding**: Grants the permissions defined in a `Role` or `ClusterRole` to a user, group, or service account within a namespace.
- **ClusterRoleBinding**: Grants the permissions defined in a `ClusterRole` to subjects cluster-wide.

## Subjects

Permissions can be assigned to:
- **User**: Identified by a username string (e.g., `oidc:alice@example.com`).
- **Group**: Identified by group name strings (e.g., `system:authenticated`, `oidc:admins`).
- **ServiceAccount**: Identities for workloads running inside the cluster or for system components.

## Aggregated ClusterRoles

Aggregated ClusterRoles allow you to combine rules from multiple ClusterRoles into one. This is useful for extending built-in roles (`admin`, `edit`, `view`) with permissions for new custom resources.

See [Aggregated ClusterRoles](./aggregated-clusterroles.md) for details.

## Authentication Methods

Tugboat supports several authentication methods that provide identity for RBAC:

- **Opaque Bearer Tokens**: Legacy tokens stored as Secrets.
- **Signed JWT Tokens**: Modern, short-lived tokens with audience and expiry. See [Service Account Tokens](./service-account-tokens.md).
- **OIDC Integration**: Support for external identity providers like Dex or Keycloak. See [OIDC Integration](./oidc.md).
- **Client Certificates**: Mutual TLS (mTLS) for system components.

## Audit Logging

All API requests and RBAC decisions can be logged for security auditing. See [Audit Logging](./audit-logging.md).

## Built-in Roles

Tugboat comes with several built-in ClusterRoles:

- `cluster-admin`: Full access to all resources in the cluster.
- `admin`: Full access within a namespace, excluding RBAC and namespace management.
- `edit`: Read/write access to most resources in a namespace.
- `view`: Read-only access to resources in a namespace.
