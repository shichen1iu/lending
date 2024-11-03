![image](https://github.com/user-attachments/assets/90bcb708-c562-4bae-90f7-ecef78b26723)# lending
## 简介
mini - lending 项目
用户可以选择存款,取款,借款,还款, 当达到清算条件之后,会触发清算

## 流程示意图
![[Pasted image 20241103211256.png]]
### 清算流程
![[Pasted image 20241103211940.png]]
## 清算条件
health_factor < 1
health_factor = collateral_amount * quilidation_threshold / borrow_amount
其中 borrow_amount = collater_amount * max_ltv
每次清算可以归还的金额为 borrow_amount * liquidation_close_factor
同时,每完成一次清算,可以获得quilidation_bonus
