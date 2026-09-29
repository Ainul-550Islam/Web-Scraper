// MOCK_ENABLED — split from mock.ts; barrel re-exports keep every public path & name


export const MOCK_ENABLED: boolean =
  import.meta.env.DEV && import.meta.env.VITE_MOCK === "1";
