import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./app/App";
import { PageErrorBoundary } from "./components/PageErrorBoundary";
import { ThemeProvider } from "./hooks/useTheme";
import { ToastProvider } from "./hooks/useToasts";
import { ConnectionProvider } from "./lib/connection";
import "./styles/tokens.css";
import "./styles/app.css";
import "./styles/responsive.css";

// Dev-only request interception for mock mode lives here so production
// chunks never import the mock module's fetch path. Mocks are hard-gated:
// import.meta.env.DEV && VITE_MOCK==="1" (lib/mock.ts).
if (import.meta.env.DEV && import.meta.env.VITE_MOCK === "1") {
  const realFetch = window.fetch.bind(window);
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const { mockResponse } = await import("./lib/mock");
    const url = typeof input === "string" ? input : input.toString();
    const path = url.replace(/^.*?\/api(?=\/|$)/, "");
    const hit = mockResponse(path);
    if (hit !== null) {
      const status = (hit as { error?: { code?: string } }).error?.code === "not_found" ? 404 : 200;
      return new Response(JSON.stringify(hit), {
        status,
        headers: { "content-type": "application/json" },
      });
    }
    return realFetch(input, init);
  };
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <PageErrorBoundary>
    <ThemeProvider>
      <ToastProvider>
        <ConnectionProvider>
          <App />
        </ConnectionProvider>
      </ToastProvider>
    </ThemeProvider>
    </PageErrorBoundary>
  </StrictMode>,
);
