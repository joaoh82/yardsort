import Link from "next/link";

const SCREENS = [
  {
    name: "phone-computer",
    alt: "The phone app: a computer's projects and workspaces, each with its branch, running agents and pull request",
    title: "Your workspaces",
    body: "Every project on the computer, each workspace with its branch, the agents running in it and how its pull request stands.",
  },
  {
    name: "phone-chat",
    alt: "The phone app: a conversation with Claude Code as a chat, with its steps folded and a field to type a message",
    title: "A conversation, as a chat",
    body: "What you asked, the agent's steps folded to one line, and what it wrote back. Type a message, stop it, or give it a picture.",
  },
  {
    name: "phone-permission",
    alt: "The phone app: Claude Code asks to run a command, with Deny and Allow once",
    title: "Answer what it asks",
    body: "When Claude Code asks to run a command, allow it once or deny it. A smoke test that waits for you can be let run the same way.",
  },
  {
    name: "phone-pull-request",
    alt: "The phone app: a pull request's checks by name, its review and Open on GitHub",
    title: "Pull requests",
    body: "The checks by name, the review and conflicts, as the computer last read them. Read-only.",
  },
];

export function Phone() {
  return (
    <section
      id="phone"
      aria-labelledby="h-phone"
      className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8"
    >
      <p className="mb-2.5 font-mono text-[12.5px] text-accent">On your phone</p>
      <h2
        id="h-phone"
        className="max-w-[720px] text-[26px] leading-[1.15] font-medium tracking-[-0.02em] md:text-[32px]"
      >
        Your agents, away from your desk
      </h2>
      <p className="mt-4 max-w-[720px] text-[17px] text-muted">
        Yardsort&apos;s phone app reaches the agents on your computer through a server you sign in
        to. They run on the computer, so they keep going whatever happens to the connection, and a
        phone gets in only after you say yes at the computer.
      </p>
      <p className="mt-4 max-w-[720px] rounded-[10px] border border-line bg-surface px-4 py-3 text-[15px]">
        <span className="font-medium">Invite only for now.</span>{" "}
        <span className="text-muted">
          The app will be in the app stores in a few weeks.{" "}
          <a href="mailto:hello@yardsort.sh" className="link">
            Write to us
          </a>{" "}
          if you would like to try it before then.
        </span>
      </p>
      <div className="mt-8 grid grid-cols-2 gap-x-6 gap-y-8 md:grid-cols-4">
        {SCREENS.map((screen) => (
          <div key={screen.name}>
            <div className="overflow-hidden rounded-[22px] border border-line bg-surface md:rounded-[26px]">
              {/* eslint-disable-next-line @next/next/no-img-element -- static export, images are plain files */}
              <img
                src={`/docs-images/${screen.name}.png`}
                alt={screen.alt}
                width={786}
                height={1704}
                loading="lazy"
                className="block h-auto w-full"
              />
            </div>
            <h3 className="mt-4 text-[17px] font-medium">{screen.title}</h3>
            <p className="mt-1.5 text-[15px] text-muted">{screen.body}</p>
          </div>
        ))}
      </div>
      <p className="mt-6 text-sm text-muted">
        Part of Remote, which is still under development and off until you turn it on. Whoever runs
        the server can see what passes through it.{" "}
        <Link href="/docs/guide/phone/" className="link">
          Read about the phone app →
        </Link>
      </p>
    </section>
  );
}
