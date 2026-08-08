import { createRoot } from "react-dom/client";
import { Overlay } from "./Overlay";
import "../styles.css";

// No StrictMode here: the overlay's canvas loop and event subscriptions are
// long-lived, and double-mounting them in dev produces a duplicated trail.
createRoot(document.getElementById("root")!).render(<Overlay />);
