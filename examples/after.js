function calculateTotal(items, discount = 0) {
  let total = 0;
  for (const item of items) {
    if (item.price > 0) total += item.price;
    if (item.tax > 0) total += item.tax;
  }
  // Aplicar desconto percentual
  if (discount > 0) total *= (1 - discount / 100);
  return Math.round(total * 100) / 100;
}

function formatPrice(value, currency = "BRL") {
  return new Intl.NumberFormat("pt-BR", {
    style: "currency",
    currency,
  }).format(value);
}

// v2: discount support & proper locale formatting
const cart = [
  { name: "Mouse", price: 150, tax: 30 },
  { name: "Teclado", price: 250, tax: 50 },
  { name: "Monitor", price: 1200, tax: 240 },
];
console.log(formatPrice(calculateTotal(cart, 10))); // 10% de desconto
console.log(formatPrice(calculateTotal(cart)));      // sem desconto
