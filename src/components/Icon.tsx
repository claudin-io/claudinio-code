import { For, type Component } from "solid-js";

const PATHS: Record<string, string[]> = {
  folder: [
    "M4 4h6v2H4zm0 14h16v2H4zM20 8h2v10h-2zM2 6h2v12H2zm8 0h10v2H10z",
  ],
  "folder-open": [
    "M4 4h6v2H4zm6 6h8v2H10zm-4 4h12v2H6zm-2 4h14v2H4zM20 8h2v10h-2zM2 6h2v12H2z",
  ],
  file: [
    "M6 4H4v16h2zm10-2H6v2h10zm4 4h-2v14h2zm-2 14H6v2h12zM16 4h2v2h-2zm-4 0h2v6h-2z",
    "M12 8h6v2h-6z",
  ],
  "chevron-left": [
    "m14 7l-5 5l5 5V7z",
  ],
  "chevron-right": [
    "m10 17l5-5l-5-5v10z",
  ],
  "chevron-down": [
    "M13 16h-2v-2h2v2Zm-2-2H9v-2h2v2Zm4 0h-2v-2h2v2Zm-6-2H7v-2h2v2Zm8 0h-2v-2h2v2ZM7 10H5V8h2v2Zm12 0h-2V8h2v2Z",
  ],
  settings: [
    "m9.25 22l-.4-3.2q-.325-.125-.612-.3t-.563-.375L4.7 19.375l-2.75-4.75l2.575-1.95Q4.5 12.5 4.5 12.338v-.675q0-.163.025-.338L1.95 9.375l2.75-4.75l2.975 1.25q.275-.2.575-.375t.6-.3l.4-3.2h5.5l.4 3.2q.325.125.613.3t.562.375l2.975-1.25l2.75 4.75l-2.575 1.95q.025.175.025.338v.674q0 .163-.05.338l2.575 1.95l-2.75 4.75l-2.95-1.25q-.275.2-.575.375t-.6.3l-.4 3.2zm2.8-6.5q1.45 0 2.475-1.025T15.55 12t-1.025-2.475T12.05 8.5q-1.475 0-2.488 1.025T8.55 12t1.013 2.475T12.05 15.5",
  ],
  send: [
    "M4 19h4v2H2v-8h2v6Zm8 0H8v-2h4v2Zm4-2h-4v-2h4v2Zm4-2h-4v-2h4v2Zm-10-2H4v-2h6v2Zm12 0h-2v-2h2v2ZM8 5H4v6H2V3h6v2Zm12 6h-4V9h4v2Zm-4-2h-4V7h4v2Zm-4-2H8V5h4v2Z",
  ],
  check: [
    "M10 18H8v-2h2v2Zm-2-2H6v-2h2v2Zm4-2v2h-2v-2h2Zm-6 0H4v-2h2v2Zm8 0h-2v-2h2v2Zm2-2h-2v-2h2v2Zm2-2h-2V8h2v2Zm2-2h-2V6h2v2Z",
  ],
  x: [
    "M9 21H7V17H9V21ZM17 21H15V17H17V21ZM11 17H9V13H11V17ZM15 17H13V13H15V17ZM13 13H11V11H13V13ZM11 11H9V7H11V11ZM15 11H13V7H15V11ZM9 7H7V3H9V7ZM17 7H15V3H17V7Z",
  ],
  search: [
    "M22 22h-2v-2h2v2Zm-2-2h-2v-2h2v2Zm-6-2H6v-2h8v2Zm4 0h-2v-2h2v2ZM6 16H4v-2h2v2Zm10 0h-2v-2h2v2ZM4 14H2V6h2v8Zm14 0h-2V6h2v8ZM6 6H4V4h2v2Zm10 0h-2V4h2v2Zm-2-2H6V2h8v2Z",
  ],
  terminal: [
    "M4 2h16v2H4zm0 18h16v2H4zM2 4h2v16H2zm18 0h2v16h-2zM6 16h2v2H6zm2-2h2v2H8zm-2-2h2v2H6z",
  ],
  pencil: [
    "M4 16H6V18H8V20H10V22H2V14H4V16ZM12 20H10V18H12V20ZM14 18H12V16H14V18ZM10 16H8V14H10V16ZM16 16H14V14H16V16ZM6 14H4V12H6V14ZM12 14H10V12H12V14ZM18 14H16V12H18V14ZM8 12H6V10H8V12ZM14 12H12V10H14V12ZM20 12H18V10H20V12ZM10 10H8V8H10V10ZM18 10H16V8H18V10ZM22 10H20V8H22V10ZM12 8H10V6H12V8ZM16 8H14V6H16V8ZM20 8H18V6H20V8ZM14 6H12V4H14V6ZM18 6H16V4H18V6ZM16 4H14V2H16V4Z",
  ],
  brain: [
    "M9 4h6v2H9zM7 6h2v2H7zm8 0h2v2h-2zm4-2h2v2h-2zm2-2h2v2h-2zM0 10h3v2H0zm21 0h3v2h-3zM3 4h2v2H3zM1 2h2v2H1zm6 12h2v2H7zm8 0h2v2h-2zM5 8h2v6H5zm12 0h2v6h-2zm-8 8h6v2H9zm0 4h6v2H9zm0-2h2v2H9zm4 0h2v2h-2zM11 0h2v3h-2z",
  ],
  "book-open": [
    "M4 4h8v16H4z",
    "M12 4h8v16h-8z",
    "M11 4h2v16h-2z",
    "M6 8h5v1H6z",
    "M6 11h5v1H6z",
    "M6 14h5v1H6z",
    "M13 8h5v1h-5z",
    "M13 11h5v1h-5z",
    "M13 14h5v1h-5z",
  ],
  loader: [
    "M13 22h-2v-6h2v6Zm-6-3H5v-2h2v2Zm12 0h-2v-2h2v2ZM9 17H7v-2h2v2Zm8 0h-2v-2h2v2Zm-9-4H2v-2h6v2Zm14 0h-6v-2h6v2ZM9 9H7V7h2v2Zm8 0h-2V7h2v2Zm-4-1h-2V2h2v6ZM7 7H5V5h2v2Zm12 0h-2V5h2v2Z",
  ],
  "notebook-pen": [
    "M13.4 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-7.4",
    "M2 6h4",
    "M2 10h4",
    "M2 14h4",
    "M2 18h4",
    "M21.378 5.626a1 1 0 1 0-3.004-3.004l-5.01 5.012a2 2 0 0 0-.506.854l-.837 2.87a.5.5 0 0 0 .62.62l2.87-.837a2 2 0 0 0 .854-.506z",
  ],
  "arrow-left": [
    "M20 11v2H4v-2zM8 13v2H6v-2zm2 2v2H8v-2zm2 2v2h-2v-2zm-4-6V9H6v2z",
    "M10 15V7H8v8zm2 2V5h-2v12z",
  ],
  "alert-circle": [
    "M9 0h6v2H9zm6 24H9v-2h6zM0 15V9h2v6zm24-6v6h-2V9zM9 2h2v4H9zm6 20h-2v-4h2zM2 15v-2h4v2zm20-6v2h-4V9zm-9-7h2v4h-2zm-2 20H9v-4h2zM2 11V9h4v2zm20 2v2h-4v-2zM7 4h2v2H7zm10 0h-2v2h2zm0 16h-2v-2h2zM7 20h2v-2H7zM2 2h5v2H2zm20 0h-5v2h5zm0 20h-5v-2h5zM2 22h5v-2H2z",
    "M2 2h2v5H2zm20 0h-2v5h2zm0 20h-2v-5h2zM2 22h2v-5H2zM4 7h2v2H4zm16 0h-2v2h2zm0 10h-2v-2h2zM4 17h2v-2H4zm6-9h4v2h-4zm0 6h4v2h-4zm-2-4h2v4H8zm6 0h2v4h-2z",
  ],
  "alert-triangle": [
    // pixel:exclamation-triangle-solid
    "M22 20v-2h-1v-2h-1v-2h-1v-2h-1v-2h-1V8h-1V6h-1V4h-1V2h-1V1h-2v1h-1v2H9v2H8v2H7v2H6v2H5v2H4v2H3v2H2v2H1v2h1v1h20v-1h1v-2zM10 11h4v3h-1v3h-2v-3h-1zM11 18h2v2h-2z",
  ],
  "alert-triangle-filled": [
    "M12 17q.425 0 .713-.288T13 16t-.288-.712T12 15t-.712.288T11 16t.288.713T12 17m-1-4h2V7h-2zm1 10.3L8.65 20H4v-4.65L.7 12L4 8.65V4h4.65L12 .7L15.35 4H20v4.65L23.3 12L20 15.35V20h-4.65zm0-2.8l2.5-2.5H18v-3.5l2.5-2.5L18 9.5V6h-3.5L12 3.5L9.5 6H6v3.5L3.5 12L6 14.5V18h3.5zm0-8.5",
  ],
  plus: [
    "M13 11h7v2h-7v7h-2v-7H4v-2h7V4h2v7Z",
  ],
  // codicon:thinking by Microsoft (16×16 — viewBox prop handles scaling)
  // https://github.com/microsoft/vscode-codicons
  "thinking-face": [
    "M9.813 1c1.172 0 2.139.872 2.291 2.002a2.5 2.5 0 0 1 1.467 4.442A3 3 0 0 1 15 10.001v.25a2.75 2.75 0 0 1-2.375 2.724l-.084.271a2.5 2.5 0 0 1-2.386 1.755H10a2.5 2.5 0 0 1-2-1c-.456.607-1.182 1-2 1h-.155a2.5 2.5 0 0 1-2.386-1.755l-.084-.271A2.75 2.75 0 0 1 1 10.25V10c0-1.082.572-2.029 1.429-2.557a2.5 2.5 0 0 1 1.467-4.442a2.313 2.313 0 0 1 4.103-1.126A2.3 2.3 0 0 1 9.811 1zM6.188 2c-.725 0-1.312.588-1.312 1.312V3.5a.5.5 0 0 1-.5.5h-.375a1.5 1.5 0 0 0-.077 2.998L4.001 7h.5a.5.5 0 0 1 0 1h-.5l-.103.002a2 2 0 0 0-1.897 1.999v.25c0 .966.783 1.75 1.75 1.75c.192 0 .364.109.447.277l.03.073l.187.596a1.5 1.5 0 0 0 1.432 1.054h.155a1.5 1.5 0 0 0 1.5-1.5V3.312C7.502 2.587 6.914 2 6.19 2zm3.625 0c-.725 0-1.312.588-1.312 1.312v9.189a1.5 1.5 0 0 0 1.5 1.5h.155c.656 0 1.236-.428 1.432-1.053l.187-.597l.03-.074a.5.5 0 0 1 .447-.276a1.75 1.75 0 0 0 1.75-1.75V10a2 2 0 0 0-1.897-1.999L12.002 8h-.5a.5.5 0 0 1 0-1h.5l.077-.002A1.5 1.5 0 0 0 12.002 4h-.375a.5.5 0 0 1-.5-.5v-.188c0-.725-.588-1.312-1.312-1.312z",
  ],
  // carbon:tool-box (24×24)
  "construction-worker": [
    "M27 9h-3V6a2 2 0 0 0-2-2H10a2 2 0 0 0-2 2v3H5a3 3 0 0 0-3 3v14a2 2 0 0 0 2 2h24a2 2 0 0 0 2-2V12a3 3 0 0 0-3-3M10 6h12v3H10Zm18 20H4v-9h8v5h8v-5h8Zm-14-9h4v3h-4ZM4 15v-3a1 1 0 0 1 1-1h22a1 1 0 0 1 1 1v3Z",
  ],
  clock: [
    "M6 2h12v2H6zM2 6h2v12H2zm18 0h2v12h-2zm-2-2h2v2h-2zM4 4h2v2H4zm2 18h12v-2H6zm12-2h2v-2h-2zM4 20h2v-2H4zm7-14h2v7h-2zm2 7h2v2h-2zm2 2h2v2h-2z",
  ],
  layers: [
    "M4 2h16v2H4zm0 18h16v2H4zM2 4h2v16H2zm18 0h2v16h-2zm-9 5h2V7h-2zm0 8h2v-6h-2z",
  ],
  "magic-button-outline": [
    "M10 14.175L11 12l2.175-1L11 10l-1-2.175L9 10l-2.175 1L9 12l1 2.175ZM10 19l-2.5-5.5L2 11l5.5-2.5L10 3l2.5 5.5L18 11l-5.5 2.5L10 19Zm8 2l-1.25-2.75L14 17l2.75-1.25L18 13l1.25 2.75L22 17l-2.75 1.25L18 21Zm-8-10Z",
  ],
  "external-link": [
    "M11 5H5v2h6V5ZM5 7H3v12h2V7Zm12 12H5v2h12v-2Zm2-6h-2v6h2v-6Zm-8 0H9v2h2v-2Zm2-2h-2v2h2v-2Zm2-2h-2v2h2V9Zm2-2h-2v2h2V7Zm2-2h-2v2h2V5Zm2-2h-2v8h2V3Z",
    "M21 3h-8v2h8V3Z",
  ],
  compress: [
    "M4 13h16v-2H4zm7-8h2V3h-2zM9 7h4V5H9zm4 0h2V5h-2zm2 2h2V7h-2zM7 9h8V7H7zm4 10h2v2h-2zm-2-2h4v2H9zm4 0h2v2h-2zm2-2h2v2h-2zm-8 0h8v2H7z",
  ],
  paperclip: [
    "M21 4v5h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1H8v-1H7v-1H6v-3h1v-1h1v-1h1v-1h1v-1h1V9h1V8h1V7h1V6h1V5h1v1h1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1H9v1H8v1h1v1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1V9h1V8h1V5h-1V4h-1V3h-3v1h-1v1h-1v1h-1v1h-1v1H9v1H8v1H7v1H6v1H5v1H4v5h1v1h1v1h1v1h5v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h2v2h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1h-1v1H7v-1H5v-1H4v-1H3v-2H2v-6h1v-1h1v-1h1V9h1V8h1V7h1V6h1V5h1V4h1V3h1V2h2V1h4v1h1v1h1v1z",
  ],
  "stop": [
    "M23 2v20h-1v1h-7v-1h-1V2h1V1h7v1zM9 2h1v20H9v1H2v-1H1V2h1V1h7z",
  ],
  "file-text": [
    "M4 2h10l6 6v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V2Zm2 0v20h12V8h-4V2H6Zm2 14h8v2H8v-2Zm0-4h8v2H8v-2Zm0-4h5v2H8V8Z",
  ],
  image: [
    "M2 3h20v18H2V3Zm2 2v9.5l5.5-5.5 6 6 3-3L22 17V5H4Zm0 14h16v-2.5l-3-3-6 6-5.5-5.5L4 17.5V19Zm4-9a2 2 0 1 1 0 4 2 2 0 0 1 0-4Z",
  ],
  "circle-outline": [
    "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8z",
  ],
  "check-circle": [
    "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z",
  ],
  refresh: [
    "M17.65 6.35A7.958 7.958 0 0 0 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08A5.99 5.99 0 0 1 12 18c-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z",
  ],
  // Compression / compaction icons
  "package": [
    "M12.876.64V.639l8.25 4.763c.541.313.875.89.875 1.515v9.525a1.75 1.75 0 0 1-.875 1.516l-8.25 4.762a1.748 1.748 0 0 1-1.75 0l-8.25-4.763a1.75 1.75 0 0 1-.875-1.515V6.917c0-.625.334-1.202.875-1.515L11.126.64a1.748 1.748 0 0 1 1.75 0Zm-1 1.298L4.251 6.34l7.75 4.474 7.75-4.474-7.625-4.402a.248.248 0 0 0-.25 0Zm.875 19.123 7.625-4.402a.25.25 0 0 0 .125-.216V7.639l-7.75 4.474ZM3.501 7.64v8.803c0 .09.048.172.125.216l7.625 4.402v-8.947Z",
  ],
  "package-process": [
    "M11 22c-.818 0-1.6-.33-3.163-.99C3.946 19.366 2 18.543 2 17.16V7m9 15V11.355M11 22c.34 0 .646-.057 1-.172M20 7v4.5M18 18l.906-.905M22 18a4 4 0 1 0-8 0a4 4 0 0 0 8 0M7.326 9.691L4.405 8.278C2.802 7.502 2 7.114 2 6.5s.802-1.002 2.405-1.778l2.92-1.413C9.13 2.436 10.03 2 11 2s1.871.436 3.674 1.309l2.921 1.413C19.198 5.498 20 5.886 20 6.5s-.802 1.002-2.405 1.778l-2.92 1.413C12.87 10.564 11.97 11 11 11s-1.871-.436-3.674-1.309M5 12l2 1m9-9L6 9",
  ],
  // octicon:goal-16 by GitHub (16×16) — pending golden goal
  goal: [
    "m13.637 2.363l1.676.335c.09.018.164.084.19.173a.25.25 0 0 1-.062.249l-1.373 1.374a.88.88 0 0 1-.619.256H12.31L9.45 7.611A1.5 1.5 0 1 1 6.5 8a1.5 1.5 0 0 1 1.889-1.449l2.861-2.862V2.552c0-.232.092-.455.256-.619L12.88.559a.25.25 0 0 1 .249-.062c.089.026.155.1.173.19Z",
    "M2 8a6 6 0 1 0 11.769-1.656a.751.751 0 1 1 1.442-.413a7.502 7.502 0 0 1-12.513 7.371A7.501 7.501 0 0 1 10.069.789a.75.75 0 0 1-.413 1.442A6 6 0 0 0 2 8",
    "M5 8a3.002 3.002 0 0 0 4.699 2.476a3 3 0 0 0 1.28-2.827a.748.748 0 0 1 1.045-.782a.75.75 0 0 1 .445.61A4.5 4.5 0 1 1 8.516 3.53a.75.75 0 1 1-.17 1.49A3 3 0 0 0 5 8",
  ],
  // lucide:goal (24×24, stroke) — golden goal achieved
  "goal-achieved": [
    "M12 13V2l8 4l-8 4",
    "M20.561 10.222a9 9 0 1 1-12.55-5.29",
    "M8.002 9.997a5 5 0 1 0 8.9 2.02",
  ],
  "package-out-of-stock": [
    "M12 22c-.818 0-1.6-.33-3.163-.988C4.946 19.373 3 18.554 3 17.175V7.542M12 22c.818 0 1.6-.33 3.163-.988C19.054 19.373 21 18.554 21 17.175V7.542M12 22v-9.97m9-4.488c0 .613-.802 1-2.405 1.773l-2.92 1.41c-1.804.87-2.705 1.304-3.675 1.304m9-4.487c0-.612-.802-.999-2.405-1.772L17 5M3 7.542c0 .613.802 1 2.405 1.773l2.92 1.41c1.804.87 2.705 1.304 3.675 1.304M3 7.542c0-.612.802-.999 2.405-1.772L7 5m-1 8.026l2 .997",
    "m10 2l2 2m0 0l2 2m-2-2l-2 2m2-2l2-2",
  ],
  "git-branch": [
    "M4 14h4v2H4zm0 6h4v2H4zm-2-4h2v4H2zm6 0h2v4H8zm8-14h4v2h-4zm0 6h4v2h-4zm-2-4h2v4h-2zm6 0h2v4h-2zm-8 13h5v2h-5zm5-5h2v5h-2zM5 2h2v10H5z",
  ],
  "git-commit": [
    "M8 5.75a2.25 2.25 0 1 1 0 4.5 2.25 2.25 0 0 1 0-4.5z",
    "M8 10.75v3.5",
    "M8 1.75v3.5",
  ],
  diff: [
    // bi:file-earmark-diff
    "M8 5a.5.5 0 0 1 .5.5V7H10a.5.5 0 0 1 0 1H8.5v1.5a.5.5 0 0 1-1 0V8H6a.5.5 0 0 1 0-1h1.5V5.5A.5.5 0 0 1 8 5m-2.5 6.5A.5.5 0 0 1 6 11h4a.5.5 0 0 1 0 1H6a.5.5 0 0 1-.5-.5",
    "M14 14V4.5L9.5 0H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2M9.5 3A1.5 1.5 0 0 0 11 4.5h2V14a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V2a1 1 0 0 1 1-1h5.5z",
  ],
  // Theme toggle icons
  monitor: [
    // monitor — pixel-art computer screen with stand
    "M2 2h20v2h-20z",
    "M2 4h2v14h-2z",
    "M20 4h2v14h-2z",
    "M2 18h20v2h-20z",
    "M10 20h4v2h-4z",
    "M7 22h10v2H7z",
  ],
  moon: [
    // crescent moon — right-side illuminated crescent as horizontal pixel blocks
    "M17 3h4v2h-4z",
    "M15 5h6v2h-6z",
    "M14 7h7v2h-7z",
    "M13 9h8v2h-8z",
    "M13 11h8v2h-8z",
    "M13 13h8v2h-8z",
    "M13 15h8v2h-8z",
    "M14 17h7v2h-7z",
    "M15 19h6v2h-6z",
    "M17 21h4v2h-4z",
  ],
  sun: [
    // Sun — blocky circle with 8 pixel rays
    // Center circle (pixelated)
    "M9 8h6v2H9z",
    "M8 10h8v4H8z",
    "M9 14h6v2H9z",
    // Orthogonal rays (N, S, W, E)
    "M11 5h2v3h-2z",
    "M11 16h2v3h-2z",
    "M5 11h3v2H5z",
    "M16 11h3v2h-3z",
    // Diagonal corner rays (NW, NE, SW, SE)
    "M7 7h2v2H7z",
    "M15 7h2v2h-2z",
    "M7 15h2v2H7z",
    "M15 15h2v2h-2z",
  ],
  // Dinkie Icons by atelierAnchor (12×12)
  // https://github.com/atelier-anchor/dinkie-icons/blob/main/LICENSE
  "speech-balloon": [
    "M1 12h3v-1H3V9H2v2H1Zm0-3h1V8H1Zm3 2h1v-1H4ZM0 8h1V3H0Zm5 2h4V9H5Zm0-2h1V7H5ZM1 3h1V2H1Zm4 3h1V5H5Zm4 3h1V8H9ZM6 5h1V3H4v1h2ZM2 2h7V1H2Zm8 6h1V3h-1ZM9 3h1V2H9Zm0 0",
  ],
  // Pixelarticons by Gerrit Halfmann — speech bubble for support
  // https://github.com/halfmage/pixelarticons
  "speech-balloon-alt": [
    "M20 2H2v20h2V4h16v12H6v2H4v2h2v-2h16V2z",
  ],
  // Game Icons by GameIcons (512×512)
  // https://github.com/game-icons/icons/blob/master/license.txt
  "file-outline-scan": [
    "M329.1 466c-5.3 1-10.8 1-16.6 1l-.5 16h19.1zm-50.8-2l-1.6 17c6.4 1 12.6 1 18.6 2l.6-17c-5.7 0-11.6-1-17.6-2m93.6-5c-8 2-16.7 4-26.2 6l1.9 16c10.4-1 19.9-3 28.7-6zm15.7-6l4.4 16c5-2 9.7-4 14.1-7l7 12l14.4-8l-7.7-13c4.1-3 7.7-7 10.8-11l16.4 14l10.6-13l-17.5-14q3.15-6 5.1-12l26.3 4l2.7-16l-25.9-5c.1-1.9.2-4 .1-6.2c0-2-.2-4.1-.4-6.2l25.8-6.2l-3.9-16.2l-25.5 6.1c-1.4-4.3-3.3-8.7-5.5-13l20.1-14l-9.5-13.7l-19.5 13.5c-3.3-4.4-7-8.8-11.2-13.1l13.2-19.4l-13.8-9.4l-11.8 17.4c-4.7-3.8-9.8-7.5-15.2-11.1l10.1-20.5l-14.9-7.4l-9.6 19.4c-5.6-3-11.5-5.9-17.7-8.6l7.1-18l-15.5-6.2l-7 17.9c-4.9-1.7-9.9-3.4-15-5l7.1-22l-15.9-5.2l-7.3 22.6c-5-1.3-10.2-2.5-15.5-3.6l4.7-30.3l-16.5-2.5l-4.6 29.7q-7.8-1.35-15.9-2.4l5.1-31.5l-16.4-2.7l-5.3 32.4c-1.1-.1-2.2-.2-3.4-.3q-7.2-.75-14.1-1.5l7.5-33.1L222 226l-7.8 34.3q-9-1.65-17.4-3.6l10.8-35.4l-37-11.6l-13.3 33.7c-6.2-2.9-12-5.9-17.2-9.1l15.4-28.6l-14.7-7.9l-14.5 26.9c-4.1-3.4-7.9-6.8-11.3-10.4l16-17.8l-12.4-11.2l-14.1 15.8c-3.6-5.5-6.47-11.1-8.63-16.9c-19.13-51 18.93-107.8 81.93-111.3c.9 6.2 2.8 12.8 5.9 19.5l74.9 21.4l3.4-8.8l-31.8-11l5.4-15.7l32.6 11.2l6.6-16.6l-31.4-16.4c-13-55.1-59.6-38.9-65.6-.3c-2.5.1-5 .3-7.4.5L167.5 33L151 35l2.9 24.6c-4.2 1.1-8.3 2.3-12.3 3.8L132.2 42L117 48.7l9.5 21.7q-6.45 3.75-12.3 8.1l-13.7-18.1l-13.3 10l14.5 19.2c-3.36 3.6-6.46 7.4-9.27 11.3L72.64 87.6l-9.31 13.8l20.6 13.9c-2.06 4.3-3.81 8.7-5.23 13.1l-26.13-6l-3.74 16.3l26.24 6c-.59 4.5-.87 9-.81 13.6l-28.21 2.2l1.31 16.6l28.6-2.3c.78 4.1 1.83 8.1 3.16 12L51.44 201l7.6 14.8l26.73-13.6c2.12 3.9 4.56 7.8 7.31 11.6l-23.7 26.5l12.41 11.1l22.11-24.7c4.3 4.4 9.1 8.6 14.4 12.7l-17.5 32.5l14.6 8l16.8-31.1c5.8 3.6 12.2 6.9 19 10l-19.7 50l12 65.3v.2l10.4 56.7h-16.4l.3 17l36.1-1l-1.8-10l-11.5-62L192 272.6c5.9 1.5 12.1 2.8 18.5 3.9l-19.4 85l16.3 3.7l19.6-86.1c5.3.7 10.8 1.3 16.4 1.8c.7 0 1.4.1 2.2.2l-11.5 70.4l16.5 2.7l11.5-71.3c5.5.7 10.9 1.5 16.1 2.4l-9.1 58.2l16.5 2.6l9-57.7c5 1.1 10 2.2 14.8 3.5l-3 62.5L302 430h-17.1v17h32.8l3.5-59.7l31.8-80.5c20.6 9.1 37.1 20 49.7 31.7c18.8 17.6 28.6 37.1 29.1 55.7c.4 17.8-8.1 34.8-24.9 47.8c-5.6 4-12 8-19.3 11M154.1 339.4l32.8-107.1l-6.5-2.1l-31.6 80.2zm169.9-4.6l13.6-34.2c-3.8-1.4-7.7-2.7-11.8-4zM212.5 53.4c6.1 0 11.2 5 11.2 11.1c0 6.2-5.1 11.2-11.2 11.2c-6.2 0-11.2-5-11.2-11.2c0-6.1 5-11.1 11.2-11.1",
  ],
  // Game Icons by GameIcons (512×512)
  // https://github.com/game-icons/icons/blob/master/license.txt
  "spawn-swarm": [
    "m244 439.765l-22.63 3l8.5-148.15a68.5 68.5 0 0 0 22.33 6.7l-7.94 138.45zm28.5 7l4.37 1.32l18.3.65v-153.58a70.1 70.1 0 0 1-22.68 6.29v145.35zm-255.26 45.6h473.52l-56.07-32.23l-37.84-9.11l-46.68-19.3l-36.71 34.72l-39.41-1.4l-27.86-8.41l-41.34 5.41l-25-15.92l-10.78-18.22L85 447.515l-55.34 20.32zm148.05-334.53c-3.757-4.877-10.72-5.866-15.686-2.227s-6.122 10.575-2.604 15.627l12 16.45l16.21-16.32zm35.71 48.72l-15.6-21.29l-16.17 16.3l15.19 20.8l15.37-14.81a8.6 8.6 0 0 1 1.21-1m25.67 35L211 220.285l-16.44 15.88l16.67 22.76c2.46-6.81 7.9-12.78 15.42-17.32zm-24.9-146.42c-2.193-5.775-8.606-8.733-14.422-6.651c-5.817 2.081-8.897 8.436-6.928 14.291l14.23 39.78l20.64-9.62zm16.83 114.35l10.91 30.48a67.8 67.8 0 0 1 21.67-6.74l-11.43-31.86zm2.4-60.42l-20.64 9.62l12.46 34.83l21.18-8.15zm30 32.69l22.62-1.87l-1.72-38.52l-22.64 1.26zm.75 17l1.51 34.25a83.5 83.5 0 0 1 22.72.42l-1.61-36.54zm17.36-120.58c-.433-6.13-5.672-10.8-11.812-10.53c-6.14.272-10.947 5.385-10.838 11.53l2.05 46.5l22.64-1.31zm82.54 20.19c1.945-5.83-1.109-12.149-6.886-14.247s-12.174.788-14.424 6.507L318 124.575l21.15 8.2zm-18.29 50.4l-21.15-8.2l-15.62 43l21.41 7.45zm-55 85.06a63.8 63.8 0 0 1 21.28 7.84l12.59-34.67l-21.42-7.45zm106.42-21c5.037-3.722 6.102-10.823 2.38-15.86s-10.823-6.102-15.86-2.38l-27.18 20.08l14.41 17.55zm-68.65 50.72l28.7-21.21l-14.41-17.55l-26.69 19.72c6.79 5.16 11.27 11.71 12.38 19.01zm-53 21.46c20.78 0 36.31-9.38 36.31-17.76s-15.53-17.76-36.31-17.76s-36.31 9.38-36.31 17.76s15.47 17.72 36.26 17.72z",
  ],
  // lucide:globe (24×24, stroke) — network activity indicator
  globe: [
    "M21.54 15H17a2 2 0 0 0-2 2v4.54",
    "M7 3.34V5a3 3 0 0 0 3 3a2 2 0 0 1 2 2c0 1.1.9 2 2 2a2 2 0 0 0 2-2c0-1.1.9-2 2-2h3.17",
    "M11 21.95V18a2 2 0 0 0-2-2a2 2 0 0 1-2-2v-1a2 2 0 0 0-2-2H2.05",
    "M22 12A10 10 0 1 1 2 12a10 10 0 0 1 20 0",
  ],
  // streamline-ultimate:archive-locker-bold by Streamline (24×24)
  "archive-drawer": [
    "M21.5 8a.5.5 0 0 0 .5-.5V2a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v5.5a.5.5 0 0 0 .5.5Zm-7.24-3.18l-.17.5a1 1 0 0 1-.95.68h-2.28a1 1 0 0 1-.95-.68l-.16-.5a1 1 0 0 1 .13-.9a1 1 0 0 1 .81-.42h2.62a1 1 0 0 1 .81.42a1 1 0 0 1 .14.9M21.5 15.5a.5.5 0 0 0 .5-.5V9.5a.5.5 0 0 0-.5-.5h-19a.5.5 0 0 0-.5.5V15a.5.5 0 0 0 .5.5ZM9.88 11.42a1 1 0 0 1 .81-.42h2.62a1 1 0 0 1 .81.42a1 1 0 0 1 .14.9l-.17.5a1 1 0 0 1-.95.68h-2.28a1 1 0 0 1-.95-.68l-.16-.5a1 1 0 0 1 .13-.9M2.5 16.5a.5.5 0 0 0-.5.5v5a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5a.5.5 0 0 0-.5-.5Zm7.38 2.42a1 1 0 0 1 .81-.42h2.62a1 1 0 0 1 .81.42a1 1 0 0 1 .14.9l-.17.5a1 1 0 0 1-.95.68h-2.28a1 1 0 0 1-.95-.68l-.16-.5a1 1 0 0 1 .13-.9",
  ],
};

export type IconName = keyof typeof PATHS;

// Icons that use a non-standard viewBox (e.g. 16×16 codicon glyphs)
const VIEWBOX: Partial<Record<IconName, string>> = {
  "speech-balloon": "0 0 12 12",
  "file-outline-scan": "0 0 512 512",
  "spawn-swarm": "0 0 512 512",
  "thinking-face": "0 0 16 16",
  "construction-worker": "0 0 32 32",
  goal: "0 0 16 16",
  "git-commit": "0 0 16 16",
  diff: "0 0 16 16",
};

// Icons drawn as strokes (e.g. Lucide glyphs) — rendered stroked even when
// the caller doesn't pass the stroke prop, since filling them looks broken.
const STROKE_ICONS: Partial<Record<IconName, boolean>> = {
  "notebook-pen": true,
  globe: true,
};

export const Icon: Component<{ name: IconName; class?: string; stroke?: boolean }> = (props) => {
  const paths = PATHS[props.name];
  if (!paths) return null;
  const stroked = props.stroke ?? STROKE_ICONS[props.name] ?? false;
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="24"
      height="24"
      viewBox={VIEWBOX[props.name] ?? "0 0 24 24"}
      fill={stroked ? "none" : "currentColor"}
      stroke={stroked ? "currentColor" : undefined}
      stroke-width={stroked ? "1.5" : undefined}
      stroke-linecap={stroked ? "round" : undefined}
      stroke-linejoin={stroked ? "round" : undefined}
      class={props.class}
    >
      <For each={paths}>{(d) => <path d={d} />}</For>
    </svg>
  );
};

export function toolIcon(name: string): IconName {
  if (name === "read_file") return "file";
  if (name === "edit_file") return "pencil";
  if (name === "list_dir") return "folder";
  if (
    name === "grep" ||
    name === "code_search" ||
    name === "semantic_search" ||
    name === "symbol_lookup" ||
    name === "go_to_definition" ||
    name === "find_references" ||
    name === "web_search"
  ) {
    return "search";
  }
  if (name === "file_outline") return "file-outline-scan";
  if (name === "ask_user") return "speech-balloon";
  if (name === "tasks_get" || name === "tasks_set") return "layers";
  if (name === "write_plan" || name === "finalize_plan") return "notebook-pen";
  if (name === "spawn_agents") return "spawn-swarm";
  if (name === "enter_plan_mode" || name === "exit_plan_mode") return "goal";
  if (name.startsWith("mcp__")) return "package";
  return "terminal";
}
