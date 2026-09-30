import { Also } from "@/components/landing/also";
import { Assist } from "@/components/landing/assist";
import { Cli } from "@/components/landing/cli";
import { Harnesses } from "@/components/landing/harnesses";
import { Hero } from "@/components/landing/hero";
import { Ideas } from "@/components/landing/ideas";
import { Install } from "@/components/landing/install";
import { OpenSource } from "@/components/landing/open-source";
import { Questions } from "@/components/landing/questions";
import { JsonLd } from "@/components/json-ld";
import { homeStructuredData } from "@/lib/structured-data";
import { HOME_DESCRIPTION, HOME_TITLE, pageMetadata } from "@/lib/seo";

export const metadata = pageMetadata(HOME_TITLE, HOME_DESCRIPTION, "/");

export default function Home() {
  return (
    <>
      <JsonLd data={homeStructuredData()} />
      <Hero />
      <Install />
      <Ideas />
      <Cli />
      <Assist />
      <Harnesses />
      <Also />
      <Questions />
      <OpenSource />
    </>
  );
}
