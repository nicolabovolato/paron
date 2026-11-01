import cowsay from "@/assets/cowsay.txt?raw";

const COWSAY = cowsay.trim().split("\n");

export function randomSaying() {
  return COWSAY[Math.floor(Math.random() * COWSAY.length)];
}
