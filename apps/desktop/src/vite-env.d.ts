/// <reference types="vite/client" />

/**
 * Typed Vite env vars — merged with vite/client's `ImportMetaEnv`.
 * No `VITE_*` variables are currently used by the frontend (the cloud-relay
 * env vars were removed). If one is reintroduced, declare it in an
 * `interface ImportMetaEnv { ... }` block here so `import.meta.env` stays
 * typed instead of falling back to `any` (see src/config.ts).
 */
