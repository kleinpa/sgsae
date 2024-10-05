import init, { run } from "./app_bindgen";

async function main() {
  await init();
  document.write(run());
}

main();
