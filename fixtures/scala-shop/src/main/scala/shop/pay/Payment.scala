package shop.pay

import shop.order.Order

// Scala 2 brace syntax.
trait Valid {
  def valid: Boolean
}

class Payment(amount: Int) extends Valid {
  def valid: Boolean = amount > 0
  def refund(order: Order): Payment = {
    if (amount > 0 && order.id > 0) new Payment(-amount) else this
  }
}

object Gateway {
  def charge(payment: Payment, amount: Int): Boolean = {
    try {
      payment.valid
    } catch {
      case e: IllegalStateException => false
    }
  }
}
