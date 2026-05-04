// Copyright 2025–2026 Fernando Borretti
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

document.addEventListener("DOMContentLoaded", function () {
  // Render inline math
  document.querySelectorAll(".math-inline").forEach(function (element) {
    katex.render(element.textContent, element, {
      displayMode: false,
      throwOnError: false,
      macros: MACROS,
    });
  });
  // Render display math
  document.querySelectorAll(".math-display").forEach(function (element) {
    katex.render(element.textContent, element, {
      displayMode: true,
      throwOnError: false,
      macros: MACROS,
    });
  });
  // Initialize syntax highlighting
  if (typeof hljs !== "undefined") {
    hljs.highlightAll();
  }
  const cardContent = document.querySelector(".card-content");
  if (cardContent) {
    cardContent.style.opacity = "1";
  }

  const reveal = document.getElementById("reveal");
  if (reveal) {
    reveal.addEventListener("click", revealAnswer);
  }
});

function revealAnswer() {
  // Show the answer (basic) or swap front->back (cloze).
  document.querySelectorAll("#answer-body, #prompt-back").forEach(function (el) {
    el.classList.remove("is-hidden");
  });
  document.querySelectorAll("#prompt-front").forEach(function (el) {
    el.classList.add("is-hidden");
  });
  // Hide the Reveal button, enable and show the grade buttons.
  const reveal = document.getElementById("reveal");
  if (reveal) reveal.classList.add("is-hidden");
  document.querySelectorAll(".grades").forEach(function (el) {
    el.classList.remove("is-hidden");
  });
  document.querySelectorAll(".grades input").forEach(function (el) {
    el.disabled = false;
  });
}

document.addEventListener("keydown", function (event) {
  // Skip during text input.
  if (event.target.tagName === "INPUT" && event.target.type === "text") {
    return;
  }

  const keybindings = {
    " ": "reveal", // Space
    u: "undo",
    r: "reject",
    1: "forgot",
    2: "hard",
    3: "good",
    4: "easy",
  };

  if (keybindings[event.key]) {
    // Ignore modifiers.
    if (event.shiftKey || event.ctrlKey || event.altKey || event.metaKey) {
      return;
    }
    event.preventDefault();
    const id = keybindings[event.key];
    const node = document.getElementById(id);
    // Skip hidden or disabled controls (e.g. grade keys before Reveal).
    if (node && !node.disabled && !node.classList.contains("is-hidden")) {
      node.click();
    }
  }
});
