// RouteState — split from router.ts; barrel re-exports keep every public path & name


export interface RouteState {
  path: string;
  query: URLSearchParams;
}
