import { Harnesses } from "@/components/landing/harnesses";
import { Hero } from "@/components/landing/hero";
import { Ideas } from "@/components/landing/ideas";
import { Install } from "@/components/landing/install";
import { OpenSource } from "@/components/landing/open-source";

export default function Home() {
  return (
    <>
      <Hero />
      <Install />
      <Ideas />
      <Harnesses />
      <OpenSource />
    </>
  );
}
