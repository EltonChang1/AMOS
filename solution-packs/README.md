# AMOS solution packs

This directory contains signed, versioned workflow contracts. A solution pack
configures domain behavior; it does not grant source access or execute itself.
AMOS must validate the contract, tenant authorization, effective dates, core
compatibility, owner approval, and an Ed25519 signature from a tenant-approved
publisher before activation.

The checked-in packs are synthetic development fixtures:

- `bank-weekly-liquidity.v1.json` defines an aggregate weekly bank-liquidity
  and funding review without customer, account, PII/NPI, PAN, SAR, restricted
  supervisory, or production data. Its thresholds and approvals are synthetic
  and are not regulatory definitions, customer policy, or customer validation.
- `payment-health-regression.v1.json` preserves the legacy payment workflow as
  regression evidence only. It is not an AMOS product requirement or an
  allowed basis for bank-product claims.

`trust/development-fixtures.json` trusts only the public key used to sign these
two synthetic fixtures and restricts it to their fixture tenants. Never add
that publisher to a pilot or production trust store. Customer packs require a
customer-approved publisher key, tenant allowlist, owner approvals, source and
policy definitions, and a separate change-control record.

See [`../docs/SOLUTION_PACK_AUTHORING.md`](../docs/SOLUTION_PACK_AUTHORING.md)
for the contract and signing workflow.
