from sqlalchemy.orm import declarative_base

Base = declarative_base()


class Order(Base):
    __tablename__ = "orders"


class OrderLine(Base):
    __tablename__ = "order_lines"
