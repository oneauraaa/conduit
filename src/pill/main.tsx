import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Pill } from "./Pill";
import "../styles.css";

// The pill always renders dark — it sits over arbitrary desktop content, so it
// needs its own contrast rather than following the app theme.
document.documentElement.dataset.theme = "dark";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Pill />
  </StrictMode>,
);
