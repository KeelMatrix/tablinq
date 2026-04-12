const composer = document.querySelector("[data-slot='composer']");
const submit = document.querySelector("[data-action='submit']");
const message = document.querySelector("[data-slot='assistant-message'] p");

submit?.addEventListener("click", () => {
  if (!composer || !message) {
    return;
  }

  const prompt = composer.value.trim();
  message.textContent = prompt
    ? `Fake response for: ${prompt}`
    : "Fake response for an empty prompt.";
});
