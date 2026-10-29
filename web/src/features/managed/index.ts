// managed: barrel — the managed scrape API's console surface.
//
// Two pages, one client. `keys` owns the key table and the typed operator
// client for `/v1/admin/*`; `usage` owns the metering view. Neither page
// talks to a customer endpoint — `/v1/scrape`, `/v1/jobs` and `/v1/usage`
// authenticate with a tenant key, which no console user has.

export * from "./keys";
export * from "./usage";
