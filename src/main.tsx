import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import RegionSelector from "./RegionSelector";
import "./styles.css";

const isSelector = new URLSearchParams(window.location.search).has("selector");
if (isSelector) document.documentElement.classList.add("selector-document");

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {isSelector ? <RegionSelector /> : <App />}
  </StrictMode>,
);
