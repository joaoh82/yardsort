import { HOME_DESCRIPTION, pageUrl, SOCIAL_IMAGE } from "./seo";
import { appVersion, RELEASES_URL, REPO_URL, repoFile } from "./site";

export function homeStructuredData() {
  const url = pageUrl("/");
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "WebSite",
        "@id": `${url}#website`,
        name: "Yardsort",
        url,
        inLanguage: "en",
        mainEntity: { "@id": `${url}#software` },
      },
      {
        "@type": "SoftwareApplication",
        "@id": `${url}#software`,
        name: "Yardsort",
        url,
        description: HOME_DESCRIPTION,
        applicationCategory: "DeveloperApplication",
        operatingSystem: "Linux, macOS, Windows",
        softwareVersion: appVersion(),
        softwareRequirements: "Git and an installed, authenticated coding agent CLI",
        license: repoFile("LICENSE"),
        sameAs: REPO_URL,
        downloadUrl: RELEASES_URL,
        image: SOCIAL_IMAGE.url,
        offers: { "@type": "Offer", price: 0, priceCurrency: "USD" },
      },
    ],
  };
}
