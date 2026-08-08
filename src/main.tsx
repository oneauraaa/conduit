import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { bootTheme } from "./lib/theme";
import "./styles.css";

bootTheme();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
