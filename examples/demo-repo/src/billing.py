import stripe

stripe.api_key = "sk_test"


def charge_customer(customer_id, amount):
    return stripe.Charge.create(amount=amount, currency="usd", customer=customer_id)


def create_order(items):
    return stripe.Order.create(items=items)


def list_customers(client):
    return client.customers.list(limit=100)
