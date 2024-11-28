import {
  TransactionConfirmationStatus,
  AccountInfo,
  Keypair,
  PublicKey,
  Transaction,
  RpcResponseAndContext,
  Commitment,
  TransactionSignature,
  SignatureStatusConfig,
  SignatureStatus,
  GetVersionedTransactionConfig,
  GetTransactionConfig,
  VersionedTransaction,
  SimulateTransactionConfig,
  SimulatedTransactionResponse,
  TransactionReturnData,
  TransactionError,
  SignatureResultCallback,
  ClientSubscriptionId,
  Connection as SolanaConnection,
  SystemProgram,
  Blockhash,
  LogsFilter,
  LogsCallback,
  AccountChangeCallback,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import {
  ProgramTestContext,
  BanksClient,
  BanksTransactionResultWithMeta,
  Clock,
} from "solana-bankrun";
import { BankrunProvider } from "anchor-bankrun";
import bs58 from "bs58";
import { BN, Wallet } from "@coral-xyz/anchor";
import { Account, unpackAccount } from "@solana/spl-token";

// 定义连接类型，可以是 Solana 实际连接或测试环境连接
export type Connection = SolanaConnection | BankrunConnection;

// 定义测试环境交易元数据的标准化格式
type BankrunTransactionMetaNormalized = {
  logMessages: string[]; // 交易日志消息
  err: TransactionError; // 交易错误信息
};

// 定义测试环境交易响应格式
type BankrunTransactionRespose = {
  slot: number; // 交易所在的槽位
  meta: BankrunTransactionMetaNormalized; // 交易元数据
};

/**
 * 测试环境上下文的包装类，提供便捷的交互方法
 */
export class BankrunContextWrapper {
  public readonly connection: BankrunConnection; // 测试环境连接实例
  public readonly context: ProgramTestContext; // 程序测试上下文
  public readonly provider: BankrunProvider; // Bankrun 提供者实例
  public readonly commitment: Commitment = "confirmed"; // 默认确认级别

  constructor(context: ProgramTestContext) {
    this.context = context;
    this.provider = new BankrunProvider(context);
    this.connection = new BankrunConnection(
      this.context.banksClient,
      this.context
    );
  }

  /**
   * 发送交易到测试环境
   * @param tx 要发送的交易对象
   * @param additionalSigners 可选的额外签名者数组
   * @returns 返回交易签名字符串
   */
  async sendTransaction(
    tx: Transaction,
    additionalSigners?: Keypair[]
  ): Promise<TransactionSignature> {
    tx.recentBlockhash = (await this.getLatestBlockhash()).toString();
    tx.feePayer = this.context.payer.publicKey;
    if (!additionalSigners) {
      additionalSigners = [];
    }
    tx.sign(this.context.payer, ...additionalSigners);
    return await this.connection.sendTransaction(tx);
  }

  /**
   * 为指定密钥对提供测试资金
   * @param keypair 要注资的密钥对或钱包
   * @param lamports 要转账的 lamports 数量
   * @returns 返回交易签名字符串
   */
  async fundKeypair(
    keypair: Keypair | Wallet,
    lamports: number | bigint
  ): Promise<TransactionSignature> {
    const ixs = [
      SystemProgram.transfer({
        fromPubkey: this.context.payer.publicKey,
        toPubkey: keypair.publicKey,
        lamports,
      }),
    ];
    const tx = new Transaction().add(...ixs);
    return await this.sendTransaction(tx);
  }

  /**
   * 获取最新的区块哈希
   * @returns 返回最新的区块哈希字符串
   */
  async getLatestBlockhash(): Promise<Blockhash> {
    const blockhash = await this.connection.getLatestBlockhash("finalized");
    return blockhash.blockhash;
  }

  /**
   * 打印指定交易的日志信息
   * @param signature 交易签名字符串
   */
  printTxLogs(signature: string): void {
    this.connection.printTxLogs(signature);
  }

  /**
   * 向前移动测试环境的时间
   * @param increment 要增加的秒数
   */
  async moveTimeForward(increment: number): Promise<void> {
    const currentClock = await this.context.banksClient.getClock();
    const newUnixTimestamp = currentClock.unixTimestamp + BigInt(increment);
    const newClock = new Clock(
      currentClock.slot,
      currentClock.epochStartTimestamp,
      currentClock.epoch,
      currentClock.leaderScheduleEpoch,
      newUnixTimestamp
    );
    await this.context.setClock(newClock);
  }

  /**
   * 设置测试环境的时间戳
   * @param unix_timestamp 要设置的 UNIX 时间戳
   */
  async setTimestamp(unix_timestamp: number): Promise<void> {
    const currentClock = await this.context.banksClient.getClock();
    const newUnixTimestamp = BigInt(unix_timestamp);
    const newClock = new Clock(
      currentClock.slot,
      currentClock.epochStartTimestamp,
      currentClock.epoch,
      currentClock.leaderScheduleEpoch,
      newUnixTimestamp
    );
    await this.context.setClock(newClock);
  }
}

/**
 * 测试环境连接类，模拟 Solana Connection 的行为
 */
export class BankrunConnection {
  private readonly _banksClient: BanksClient;
  private readonly context: ProgramTestContext;
  // 存储交易签名到交易元数据的映射
  private transactionToMeta: Map<
    TransactionSignature,
    BanksTransactionResultWithMeta
  > = new Map();
  private clock: Clock; // 测试环境时钟

  // 用于事件订阅的状态管理
  private nextClientSubscriptionId = 0;
  private onLogCallbacks = new Map<number, LogsCallback>();
  private onAccountChangeCallbacks = new Map<
    number,
    [PublicKey, AccountChangeCallback]
  >();

  constructor(banksClient: BanksClient, context: ProgramTestContext) {
    this._banksClient = banksClient;
    this.context = context;
  }

  /**
   * 获取当前槽位号
   * @returns 返回当前槽位号
   */
  getSlot(): Promise<bigint> {
    return this._banksClient.getSlot();
  }

  /**
   * 将 BankrunConnection 转换为 SolanaConnection
   * @returns 返回转换后的 SolanaConnection 对象
   */
  toConnection(): SolanaConnection {
    return this as unknown as SolanaConnection;
  }

  /**
   * 获取代币账户信息
   * @param publicKey 代币账户的公钥
   * @returns 返回解析后的代币账户信息
   */
  async getTokenAccount(publicKey: PublicKey): Promise<Account> {
    const info = await this.getAccountInfo(publicKey);
    return unpackAccount(publicKey, info, info.owner);
  }

  /**
   * 批量获取多个账户的信息
   * @param publicKeys 要查询的公钥数组
   * @param _commitmentOrConfig 可选的确认级别或配置
   * @returns 返回账户信息数组
   */
  async getMultipleAccountsInfo(
    publicKeys: PublicKey[],
    _commitmentOrConfig?: Commitment
  ): Promise<AccountInfo<Buffer>[]> {
    const accountInfos = [];
    for (const publicKey of publicKeys) {
      const accountInfo = await this.getAccountInfo(publicKey);
      accountInfos.push(accountInfo);
    }
    return accountInfos;
  }

  /**
   * 获取账户信息
   * @param publicKey 要查询的账户公钥
   */
  async getAccountInfo(
    publicKey: PublicKey
  ): Promise<null | AccountInfo<Buffer>> {
    const parsedAccountInfo = await this.getParsedAccountInfo(publicKey);
    //三元表达式
    return parsedAccountInfo ? parsedAccountInfo.value : null;
  }

  async getAccountInfoAndContext(
    publicKey: PublicKey,
    _commitment?: Commitment
  ): Promise<RpcResponseAndContext<null | AccountInfo<Buffer>>> {
    return await this.getParsedAccountInfo(publicKey);
  }

  /**
   * 发送原始交易数据
   * @param rawTransaction 原始交易数据
   * @param _options 可选的配置选项
   * @returns 返回交易签名
   */
  async sendRawTransaction(
    rawTransaction: Buffer | Uint8Array | Array<number>,
    _options?: any
  ): Promise<TransactionSignature> {
    const tx = Transaction.from(rawTransaction);
    const signature = await this.sendTransaction(tx);
    return signature;
  }

  /**
   * 发送交易
   * @param tx 要发送的交易
   * @returns 交易签名
   */
  async sendTransaction(tx: Transaction): Promise<TransactionSignature> {
    const banksTransactionMeta = await this._banksClient.tryProcessTransaction(
      tx
    );
    if (banksTransactionMeta.result) {
      throw new Error(banksTransactionMeta.result);
    }
    const signature = bs58.encode(tx.signatures[0].signature);
    this.transactionToMeta.set(signature, banksTransactionMeta);
    let finalizedCount = 0;
    while (finalizedCount < 10) {
      const signatureStatus = (await this.getSignatureStatus(signature)).value
        .confirmationStatus;
      if (signatureStatus.toString() == '"finalized"') {
        finalizedCount += 1;
      }
    }

    // update the clock slot/timestamp
    // sometimes race condition causes failures so we retry
    try {
      await this.updateSlotAndClock();
    } catch (e) {
      await this.updateSlotAndClock();
    }

    if (this.onLogCallbacks.size > 0) {
      const transaction = await this.getTransaction(signature);

      const context = { slot: transaction.slot };
      const logs = {
        logs: transaction.meta.logMessages,
        err: transaction.meta.err,
        signature,
      };
      for (const logCallback of this.onLogCallbacks.values()) {
        logCallback(logs, context);
      }
    }

    for (const [
      publicKey,
      callback,
    ] of this.onAccountChangeCallbacks.values()) {
      const accountInfo = await this.getParsedAccountInfo(publicKey);
      callback(accountInfo.value, accountInfo.context);
    }

    return signature;
  }

  /**
   * 更新测试环境的槽位和时钟
   * @private
   */
  private async updateSlotAndClock() {
    const currentSlot = await this.getSlot();
    const nextSlot = currentSlot + BigInt(1);
    this.context.warpToSlot(nextSlot);
    const currentClock = await this._banksClient.getClock();
    const newClock = new Clock(
      nextSlot,
      currentClock.epochStartTimestamp,
      currentClock.epoch,
      currentClock.leaderScheduleEpoch,
      currentClock.unixTimestamp + BigInt(1)
    );
    this.context.setClock(newClock);
    this.clock = newClock;
  }

  getTime(): number {
    return Number(this.clock.unixTimestamp);
  }

  async getParsedAccountInfo(
    publicKey: PublicKey
  ): Promise<RpcResponseAndContext<AccountInfo<Buffer>>> {
    const accountInfoBytes = await this._banksClient.getAccount(publicKey);
    if (accountInfoBytes === null) {
      return {
        context: { slot: Number(await this._banksClient.getSlot()) },
        value: null,
      };
    }
    accountInfoBytes.data = Buffer.from(accountInfoBytes.data);
    const accountInfoBuffer = accountInfoBytes as AccountInfo<Buffer>;
    return {
      context: { slot: Number(await this._banksClient.getSlot()) },
      value: accountInfoBuffer,
    };
  }

  async getLatestBlockhash(commitment?: Commitment): Promise<
    Readonly<{
      blockhash: string;
      lastValidBlockHeight: number;
    }>
  > {
    const blockhashAndBlockheight = await this._banksClient.getLatestBlockhash(
      commitment
    );
    return {
      blockhash: blockhashAndBlockheight[0],
      lastValidBlockHeight: Number(blockhashAndBlockheight[1]),
    };
  }

  async getSignatureStatus(
    signature: string,
    _config?: SignatureStatusConfig
  ): Promise<RpcResponseAndContext<null | SignatureStatus>> {
    const transactionStatus = await this._banksClient.getTransactionStatus(
      signature
    );
    if (transactionStatus === null) {
      return {
        context: { slot: Number(await this._banksClient.getSlot()) },
        value: null,
      };
    }
    return {
      context: { slot: Number(await this._banksClient.getSlot()) },
      value: {
        slot: Number(transactionStatus.slot),
        confirmations: Number(transactionStatus.confirmations),
        err: transactionStatus.err,
        confirmationStatus:
          transactionStatus.confirmationStatus as TransactionConfirmationStatus,
      },
    };
  }

  /**
   * There's really no direct equivalent to getTransaction exposed by SolanaProgramTest, so we do the best that we can here - it's a little hacky.
   */
  async getTransaction(
    signature: string,
    _rawConfig?: GetTransactionConfig | GetVersionedTransactionConfig
  ): Promise<BankrunTransactionRespose | null> {
    const txMeta = this.transactionToMeta.get(
      signature as TransactionSignature
    );
    if (txMeta === undefined) {
      return null;
    }
    const transactionStatus = await this._banksClient.getTransactionStatus(
      signature
    );
    const meta: BankrunTransactionMetaNormalized = {
      logMessages: txMeta.meta.logMessages,
      err: txMeta.result,
    };
    return {
      slot: Number(transactionStatus.slot),
      meta,
    };
  }

  findComputeUnitConsumption(signature: string): bigint {
    const txMeta = this.transactionToMeta.get(
      signature as TransactionSignature
    );
    if (txMeta === undefined) {
      throw new Error("Transaction not found");
    }
    return txMeta.meta.computeUnitsConsumed;
  }

  printTxLogs(signature: string): void {
    const txMeta = this.transactionToMeta.get(
      signature as TransactionSignature
    );
    if (txMeta === undefined) {
      throw new Error("Transaction not found");
    }
    console.log(txMeta.meta.logMessages);
  }

  async simulateTransaction(
    transaction: Transaction | VersionedTransaction,
    _config?: SimulateTransactionConfig
  ): Promise<RpcResponseAndContext<SimulatedTransactionResponse>> {
    const simulationResult = await this._banksClient.simulateTransaction(
      transaction
    );
    const returnDataProgramId =
      simulationResult.meta?.returnData?.programId.toBase58();
    const returnDataNormalized = Buffer.from(
      simulationResult.meta?.returnData?.data
    ).toString("base64");
    const returnData: TransactionReturnData = {
      programId: returnDataProgramId,
      data: [returnDataNormalized, "base64"],
    };
    return {
      context: { slot: Number(await this._banksClient.getSlot()) },
      value: {
        err: simulationResult.result,
        logs: simulationResult.meta.logMessages,
        accounts: undefined,
        unitsConsumed: Number(simulationResult.meta.computeUnitsConsumed),
        returnData,
      },
    };
  }

  onSignature(
    signature: string,
    callback: SignatureResultCallback,
    commitment?: Commitment
  ): ClientSubscriptionId {
    const txMeta = this.transactionToMeta.get(
      signature as TransactionSignature
    );
    this._banksClient.getSlot(commitment).then((slot) => {
      if (txMeta) {
        callback({ err: txMeta.result }, { slot: Number(slot) });
      }
    });
    return 0;
  }

  async removeSignatureListener(_clientSubscriptionId: number): Promise<void> {
    // Nothing actually has to happen here! Pretty cool, huh?
    // This function signature only exists to match the web3js interface
  }

  /**
   * 注册日志监听器
   * @param filter 日志过滤器
   * @param callback 回调函数
   * @param _commitment 确认级别
   * @returns 返回订阅ID
   */
  onLogs(
    filter: LogsFilter,
    callback: LogsCallback,
    _commitment?: Commitment
  ): ClientSubscriptionId {
    const subscriptId = this.nextClientSubscriptionId;
    this.onLogCallbacks.set(subscriptId, callback);
    this.nextClientSubscriptionId += 1;
    return subscriptId;
  }

  /**
   * 移除日志监听器
   * @param clientSubscriptionId 订阅ID
   */
  async removeOnLogsListener(
    clientSubscriptionId: ClientSubscriptionId
  ): Promise<void> {
    this.onLogCallbacks.delete(clientSubscriptionId);
  }

  /**
   * 注册账户变更监听器
   * @param publicKey 要监听的账户公钥
   * @param callback 回调函数
   * @param _commitment 确认级别
   * @returns 返回订阅ID
   */
  onAccountChange(
    publicKey: PublicKey,
    callback: AccountChangeCallback,
    _commitment?: Commitment
  ): ClientSubscriptionId {
    const subscriptId = this.nextClientSubscriptionId;
    this.onAccountChangeCallbacks.set(subscriptId, [publicKey, callback]);
    this.nextClientSubscriptionId += 1;
    return subscriptId;
  }

  /**
   * 移除账户变更监听器
   * @param clientSubscriptionId 订阅ID
   */
  async removeAccountChangeListener(
    clientSubscriptionId: ClientSubscriptionId
  ): Promise<void> {
    this.onAccountChangeCallbacks.delete(clientSubscriptionId);
  }

  /**
   * 获取租金豁免所需的最小余额
   * @param _ 账户大小（字节）
   * @returns 返回所需的最小余额（lamports）
   */
  async getMinimumBalanceForRentExemption(_: number): Promise<number> {
    return 10 * LAMPORTS_PER_SOL;
  }
}

/**
 * 将数值转换为 BN (Big Number) 类型
 * @param value 要转换的数值
 * @returns BN 实例
 */
export function asBN(value: number | bigint): BN {
  return new BN(Number(value));
}
