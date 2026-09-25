# Issuer authorization flags on Stellar

Evidence for section 6.1 of [`ARCHITECTURE.md`](../../docs/ARCHITECTURE.md): which assets on Stellar can be frozen or clawed back by their issuer, including when the balance is held by a Soroban contract.

| File | What it is |
|---|---|
| `snapshot-2026-09-25.json` | Flag state, thresholds and signer summary for every issuer in the table, as returned by Horizon on 25 September 2026 |
| `check_issuer_flags.sh` | Re-queries each issuer on the live ledger and reports any flag that has changed since the snapshot |

## Re-run it

```sh
./check_issuer_flags.sh
```

Requires `curl` and `jq`. Point it at testnet or a private Horizon with `HORIZON=https://... ./check_issuer_flags.sh`. Exit code 2 means at least one issuer's flags have changed.

## How the issuers were chosen

Asset codes on Stellar are permissionless: querying `BENJI` on Horizon returns 23 issuers and `USDY` returns 37, nearly all of them impostors. Each issuer here was attributed to its operator by matching the account's `home_domain` against the operator's real domain and, where published, its `/.well-known/stellar.toml`. Asset codes were never used as identity.

Flags live on the **issuer account**, not the asset, so every asset from one issuer shares one flag set.

## What the four flags mean for a coverage pool

| Flag | Effect on a balance held by our pool contract |
|---|---|
| `auth_revocable` | Issuer can freeze it at any time, including balances that already exist |
| `auth_clawback_enabled` | Issuer can remove it, but only from balance entries created after the flag was set |
| `auth_immutable` | Locks the other flags permanently; the only way an issuer's choice becomes irreversible |
| `auth_required` | Pool contract must be explicitly authorized before it can hold the asset at all |

Sources: [Stellar Asset Contract](https://developers.stellar.org/docs/tokens/stellar-asset-contract) · [Controlling asset access](https://developers.stellar.org/docs/tokens/control-asset-access) · [CAP-35](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0035.md)

## Limits of this survey

It covers classic issuers only. A token issued as a Soroban contract has no issuer account and no auth flags, and whether it can be frozen depends entirely on that contract's own admin logic. Spiko and Realiz, two of the largest RWA issuers on Stellar by value, did not resolve to classic issuers and are not in this table. See `ARCHITECTURE.md` section 12.2.
