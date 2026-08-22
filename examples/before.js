function calculateTotal(items) {
  var total = 0;
  for (var i = 0; i < items.length; i++) {
    var item = items[i];
    if (item.price > 0) {
      total = total + item.price;
    }
    if (item.tax > 0) {
      total = total + item.tax;
    }
  }
  return total;
}

function formatPrice(value) {
  return "R$ " + value.toFixed(2);
}

// old version without discount support
var cart = [
  { name: "Mouse", price: 150, tax: 30 },
  { name: "Teclado", price: 250, tax: 50 },
];
console.log(formatPrice(calculateTotal(cart)));
