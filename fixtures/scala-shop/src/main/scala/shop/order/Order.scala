package shop
package order

import shop.pay.{Payment, Gateway => G}

// Scala 3 indentation syntax.
enum Status:
  case Open, Paid

final case class Order(id: Long, payment: Payment):
  def total(items: List[Int]): Int =
    var sum = 0
    for item <- items do
      if item > 0 then sum += item
      else if item < -100 then
        sum -= 1
    sum

  def pay(): Boolean =
    val ok = G.charge(payment, 10) && payment.valid
    ok match
      case true => Order.log("paid")
      case _ => false
  end pay

object Order:
  def log(message: String): Boolean = message.nonEmpty
