/**
 * Which account a Remote sign-in is in, by the email the identity provider gave when it signed
 * in. Two sign-ins on one server can be in different accounts, when the browser was signed in as
 * someone else, and a device in another account sees nothing of this one: this is how to tell.
 */
export function AccountLine({ email }: { email: string | null }) {
  return email ? (
    <p>
      Account: <span className="text-ink select-text">{email}</span>
    </p>
  ) : (
    <p className="text-ink-faint">
      The account it signed in to is not known here: it signed in before Yardsort kept it, or the
      server did not say. It shows from the next sign-in.
    </p>
  );
}
