/**
 * One reply for a round of numbered questions. A single question is answered as it was typed;
 * several are numbered the way they were asked, and one left blank says so.
 */
export function joinAnswers(questions: string[], answers: string[]): string {
  if (questions.length <= 1) return (answers[0] ?? "").trim();
  return questions
    .map((_, index) => `${index + 1}. ${(answers[index] ?? "").trim() || "(no answer)"}`)
    .join("\n\n");
}
