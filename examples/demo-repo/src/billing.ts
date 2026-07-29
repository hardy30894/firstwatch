import Stripe from "stripe";

const stripe = new Stripe(process.env.STRIPE_KEY!);

export async function chargeCustomer(customerId: string, amount: number) {
  return stripe.charges.create({ amount, currency: "usd", customer: customerId });
}

export async function createOrder(items: string[]) {
  return stripe.orders.create({ items });
}

export async function invoiceCustomer(customerId: string) {
  return stripe.invoices.create({ customer: customerId });
}

export async function listCustomers() {
  return stripe.customers.list({ limit: 100 });
}
