import OpenAI from "openai";

const client = new OpenAI();

export async function ask(model: string, messages: object[]) {
  return client.chat.completions.create({ model, messages });
}

export async function embed(text: string) {
  return client.embeddings.create({ model: "text-embedding-3-small", input: text });
}
